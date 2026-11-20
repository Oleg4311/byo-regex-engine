// Интерфейс песочницы: вызывает собранный из Rust модуль WebAssembly (см. src/wasm.rs). Весь вывод строится через textContent.
const $ = (id) => document.getElementById(id);
const enc = new TextEncoder();
const dec = new TextDecoder();

let ex;
try {
  const res = await fetch('byo_regex.wasm');
  const bytes = await res.arrayBuffer();
  ({ instance: { exports: ex } } = await WebAssembly.instantiate(bytes, {}));
} catch (e) {
  $('error').hidden = false;
  $('error').textContent = 'Не удалось загрузить модуль WebAssembly: ' + e.message;
  throw e;
}

/** Вызывает analyze и возвращает разобранный JSON. engine: 0 — Pike, 1 — откат, 2 — откат с запоминанием. */
function analyze(pattern, hay, engine, maxSteps) {
  const p = enc.encode(pattern);
  const h = enc.encode(hay);
  const pp = ex.alloc(p.length);
  new Uint8Array(ex.memory.buffer, pp, p.length).set(p); // буфер берём заново после каждого alloc: память могла вырасти
  const hp = ex.alloc(h.length);
  new Uint8Array(ex.memory.buffer, hp, h.length).set(h);
  const out = ex.analyze(pp, p.length, hp, h.length, engine, maxSteps);
  const len = new DataView(ex.memory.buffer).getUint32(out, true);
  const json = dec.decode(new Uint8Array(ex.memory.buffer, out + 4, len));
  ex.dealloc(out, 4 + len);
  ex.dealloc(pp, p.length);
  ex.dealloc(hp, h.length);
  return JSON.parse(json);
}

/** Карта «байтовое смещение UTF-8 → индекс в строке JS (UTF-16)». */
function byteMap(text) {
  const map = new Map();
  let b = 0;
  let i = 0;
  for (const ch of text) {
    map.set(b, i);
    const cp = ch.codePointAt(0);
    b += cp < 0x80 ? 1 : cp < 0x800 ? 2 : cp < 0x10000 ? 3 : 4;
    i += ch.length;
  }
  map.set(b, i);
  return map;
}

const fmt = (n) => n.toLocaleString('ru-RU');

function render() {
  const pat = $('pat').value;
  const hay = $('hay').value;
  const engine = Number($('engine').value);
  const maxSteps = Math.max(1000, Math.min(100_000_000, Number($('steps').value) || 3_000_000));
  const err = $('error');
  err.hidden = true;
  const r = analyze(pat, hay, engine, maxSteps);
  const hl = $('hl');
  hl.replaceChildren();
  $('matches').replaceChildren();
  if (!r.ok) {
    err.hidden = false;
    err.textContent = 'Ошибка в шаблоне: ' + r.error;
    $('stats').textContent = '';
    $('prog').textContent = '';
    hl.textContent = hay;
    return;
  }
  $('prog').textContent = r.program;
  if (r.limit) {
    err.hidden = false;
    err.textContent = `Откат не уложился в ${fmt(maxSteps)} шагов и остановлен: ответа нет. Это не «нет совпадения». Попробуйте Pike VM или откат с запоминанием.`;
    hl.textContent = hay;
    $('stats').textContent = `Шагов: ${fmt(r.steps)}`;
    return;
  }
  const map = byteMap(hay);
  let cursor = 0;
  r.matches.forEach((groups, i) => {
    const [s, e] = groups[0];
    const si = map.get(s);
    const ei = map.get(e);
    if (si > cursor) hl.append(hay.slice(cursor, si));
    const mark = document.createElement('mark');
    mark.className = 'm' + (i % 2);
    mark.textContent = hay.slice(si, ei) || '∅';
    mark.title = `Совпадение ${i + 1}`;
    hl.append(mark);
    cursor = ei;
  });
  if (cursor < hay.length) hl.append(hay.slice(cursor));
  if (!hay) hl.textContent = '';
  $('stats').textContent = `Совпадений: ${r.matches.length}. Шагов выполнено: ${fmt(r.steps)}.`;

  const names = new Map(r.names.map(([n, idx]) => [idx, n]));
  r.matches.slice(0, 50).forEach((groups, i) => {
    const li = document.createElement('li');
    const parts = groups.map((g, gi) => {
      if (!g) return `${gi === 0 ? 'всё' : names.get(gi) ?? gi}: —`;
      const t = hay.slice(map.get(g[0]), map.get(g[1]));
      return `${gi === 0 ? 'всё' : names.get(gi) ?? gi}: «${t}»`;
    });
    li.textContent = `№${i + 1}, байты ${groups[0][0]}–${groups[0][1]}; ${parts.join('; ')}`;
    $('matches').append(li);
  });
  if (r.matches.length > 50) {
    const li = document.createElement('li');
    li.textContent = `…и ещё ${r.matches.length - 50}`;
    $('matches').append(li);
  }
}

const EXAMPLES = {
  date: ['(?P<year>\\d{4})-(\\d{2})-(\\d{2})', 'Релиз 2026-11-20, а перенесли с 2026-10-31.'],
  lazy: ['<.+?>', 'Жадно: <b>жирный</b> и <i>курсив</i>'],
  alt: ['a|ab|abc', 'abc abc'],
  unicode: ['[а-я]+', 'Привет, мир! Hello, world!'],
};
document.querySelectorAll('[data-ex]').forEach((b) =>
  b.addEventListener('click', () => {
    const [p, h] = EXAMPLES[b.dataset.ex];
    $('pat').value = p;
    $('hay').value = h;
    $('engine').value = '0';
    render();
  }),
);
for (const id of ['pat', 'hay', 'engine', 'steps']) $(id).addEventListener('input', render);

// ---- катастрофический откат ----
const BOOM = '(a+)+b';
const boomLimit = 3_000_000;
const text = (n) => 'a'.repeat(n) + 'c';

function boomRow(name, r, limit) {
  const tr = document.createElement('tr');
  const td = (t, cls) => {
    const c = document.createElement('td');
    c.textContent = t;
    if (cls) c.className = cls;
    return c;
  };
  tr.append(td(name), td(r.limit ? `> ${fmt(limit)}` : fmt(r.steps), 'num'), td(r.limit ? 'остановлен по пределу шагов: ответа нет' : r.matches.length ? 'совпало' : 'нет совпадения (верный ответ)'));
  return tr;
}

$('boom').addEventListener('click', () => {
  const n = Math.max(4, Math.min(40, Number($('n').value) || 22));
  const hay = text(n);
  const tbody = $('boom-table').tBodies[0];
  tbody.replaceChildren(
    boomRow('Pike VM', analyze(BOOM, hay, 0, boomLimit), boomLimit),
    boomRow('Откат (классический)', analyze(BOOM, hay, 1, boomLimit), boomLimit),
    boomRow('Откат с запоминанием', analyze(BOOM, hay, 2, boomLimit), boomLimit),
  );
  $('boom-table').caption.textContent = `Шаблон ${BOOM}, текст: ${n} × «a» и «c» (${n + 1} символов). Предел отката: ${fmt(boomLimit)} шагов.`;
});

const SVGNS = 'http://www.w3.org/2000/svg';
function el(name, attrs, text) {
  const e = document.createElementNS(SVGNS, name);
  for (const [k, v] of Object.entries(attrs)) e.setAttribute(k, v);
  if (text) e.textContent = text;
  return e;
}

$('chart').addEventListener('click', () => {
  const ns = [];
  for (let n = 4; n <= 30; n += 2) ns.push(n);
  const data = ns.map((n) => ({
    n,
    pike: analyze(BOOM, text(n), 0, boomLimit).steps,
    bt: analyze(BOOM, text(n), 1, boomLimit).steps,
    memo: analyze(BOOM, text(n), 2, boomLimit).steps,
  }));
  const svg = $('svg');
  svg.querySelectorAll(':scope > :not(title):not(desc)').forEach((x) => x.remove());
  const W = 720, H = 340, L = 64, R = 16, T = 12, B = 44;
  const maxLog = Math.ceil(Math.log10(boomLimit * 1.5));
  const x = (n) => L + ((n - 4) / 26) * (W - L - R);
  const y = (v) => T + (1 - Math.log10(Math.max(v, 1)) / maxLog) * (H - T - B);
  const axis = getComputedStyle(document.documentElement).getPropertyValue('--mute');
  for (let p = 0; p <= maxLog; p++) {
    svg.append(el('line', { x1: L, x2: W - R, y1: y(10 ** p), y2: y(10 ** p), stroke: 'currentColor', 'stroke-opacity': 0.15 }));
    svg.append(el('text', { x: L - 8, y: y(10 ** p) + 4, 'text-anchor': 'end', 'font-size': 12, fill: 'currentColor' }, fmt(10 ** p)));
  }
  for (const n of [4, 10, 16, 22, 28]) svg.append(el('text', { x: x(n), y: H - 24, 'text-anchor': 'middle', 'font-size': 12, fill: 'currentColor' }, String(n)));
  svg.append(el('text', { x: (L + W - R) / 2, y: H - 6, 'text-anchor': 'middle', 'font-size': 12, fill: 'currentColor' }, 'длина текста n (логарифмическая шкала шагов)'));
  void axis;
  const series = [
    ['pike', 'var(--c-pike)', ''],
    ['bt', 'var(--c-bt)', '8 5'],
    ['memo', 'var(--c-memo)', '2 4'],
  ];
  for (const [key, color, dash] of series) {
    const pts = data.map((d) => `${x(d.n)},${y(d[key])}`).join(' ');
    const line = el('polyline', { points: pts, fill: 'none', stroke: color, 'stroke-width': 2.5 });
    if (dash) line.setAttribute('stroke-dasharray', dash);
    svg.append(line);
    for (const d of data) svg.append(el('circle', { cx: x(d.n), cy: y(d[key]), r: 3, fill: color }));
  }
  const tb = $('chart-table').tBodies[0];
  tb.replaceChildren(
    ...data.map((d) => {
      const tr = document.createElement('tr');
      for (const v of [String(d.n), fmt(d.pike), d.bt >= boomLimit ? `> ${fmt(boomLimit)}` : fmt(d.bt), fmt(d.memo)]) {
        const td = document.createElement('td');
        td.textContent = v;
        td.className = 'num';
        tr.append(td);
      }
      return tr;
    }),
  );
  $('chart-box').hidden = false;
});

render();
