import AxeBuilder from '@axe-core/playwright';
import { expect, test } from '@playwright/test';

test.beforeEach(async ({ page }) => {
  await page.goto('./');
  await expect(page.locator('#hl mark').first()).toBeVisible(); // wasm загружен, первый поиск выполнен
});

test('пример по умолчанию: две даты подсвечены, группы и имя группы показаны', async ({ page }) => {
  await expect(page.locator('#hl mark')).toHaveCount(2);
  await expect(page.locator('#hl mark').first()).toHaveText('2026-11');
  await expect(page.locator('#stats')).toContainText('Совпадений: 2');
  await expect(page.locator('#matches li').first()).toContainText('year: «2026»');
  await expect(page.locator('#prog')).toContainText('match');
});

test('ленивый и жадный квантификатор дают разные совпадения', async ({ page }) => {
  await page.getByRole('button', { name: 'Жадность и лень' }).click();
  await expect(page.locator('#hl mark')).toHaveCount(4); // <b>, </b>, <i>, </i>
  await page.locator('#pat').fill('<.+>');
  await expect(page.locator('#hl mark')).toHaveCount(1); // жадный съедает всё от первого < до последнего >
  await expect(page.locator('#hl mark').first()).toContainText('<b>жирный</b> и <i>курсив</i>');
});

test('юникод: подсвечивается кириллица по символам, а не по байтам', async ({ page }) => {
  await page.getByRole('button', { name: 'Юникод' }).click();
  await expect(page.locator('#hl mark').first()).toHaveText('Привет'.slice(1)); // «ривет»: заглавная П вне [а-я]
  await expect(page.locator('#hl mark')).toHaveCount(2);
});

test('ошибка в шаблоне показывается с позицией и исчезает после исправления', async ({ page }) => {
  await page.locator('#pat').fill('ab(cd');
  await expect(page.locator('#error')).toBeVisible();
  await expect(page.locator('#error')).toContainText('позиция 2');
  await page.locator('#pat').fill('ab');
  await expect(page.locator('#error')).toBeHidden();
});

test('классический откат на (a+)+b упирается в предел, Pike отвечает', async ({ page }) => {
  await page.locator('#n').fill('26');
  await page.locator('#boom').click();
  const rows = page.locator('#boom-table tbody tr');
  await expect(rows).toHaveCount(3);
  await expect(rows.nth(0)).toContainText('нет совпадения (верный ответ)');
  await expect(rows.nth(1)).toContainText('остановлен по пределу шагов');
  await expect(rows.nth(2)).toContainText('нет совпадения (верный ответ)');
  const steps = async (i: number) => Number((await rows.nth(i).locator('td').nth(1).innerText()).replace(/\D/g, ''));
  expect(await steps(0)).toBeLessThan(2000); // Pike: порядка n шагов
  expect(await steps(2)).toBeLessThan(20000);
});

test('на выбранном откате без предела шагов страница честно говорит «ответа нет»', async ({ page }) => {
  await page.locator('#pat').fill('(a+)+b');
  await page.locator('#hay').fill('a'.repeat(30) + 'c');
  await page.locator('#engine').selectOption('1');
  await page.locator('#steps').fill('100000');
  await expect(page.locator('#error')).toContainText('не уложился');
  await expect(page.locator('#error')).toContainText('Это не «нет совпадения»');
});

test('график строится, данные доступны таблицей', async ({ page }) => {
  await page.getByRole('button', { name: 'Построить график' }).click();
  await expect(page.locator('#chart-box')).toBeVisible();
  await expect(page.locator('#svg polyline')).toHaveCount(3);
  await page.locator('#chart-box summary').click();
  await expect(page.locator('#chart-table tbody tr')).toHaveCount(14);
  // у отката шаги растут до предела, у Pike остаются малыми
  const last = page.locator('#chart-table tbody tr').last();
  await expect(last.locator('td').nth(2)).toContainText('>');
});

test('введённый текст не превращается в разметку', async ({ page }) => {
  await page.locator('#hay').fill('<img src=x onerror=window.__xss=1> abc');
  await page.locator('#pat').fill('.');
  await expect(page.locator('#hl mark').first()).toBeVisible();
  expect(await page.evaluate(() => (window as unknown as { __xss?: number }).__xss)).toBeUndefined();
  expect(await page.locator('#hl img').count()).toBe(0);
});

test('страница не прокручивается вбок', async ({ page }) => {
  const overflow = await page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth);
  expect(overflow).toBeLessThanOrEqual(0);
});

for (const scheme of ['light', 'dark'] as const) {
  test(`доступность axe: ${scheme} тема`, async ({ page }) => {
    await page.emulateMedia({ colorScheme: scheme });
    await page.getByRole('button', { name: 'Построить график' }).click();
    await expect(page.locator('#chart-box')).toBeVisible();
    await page.locator('#pat').fill('ab(');
    await expect(page.locator('#error')).toBeVisible();
    const res = await new AxeBuilder({ page }).withTags(['wcag2a', 'wcag2aa', 'wcag21aa']).analyze();
    expect(res.violations.map((v) => `${v.id}: ${v.nodes.map((n) => n.target.join(' ')).join(', ')}`)).toEqual([]);
  });
}

test('version.json содержит версию', async ({ request, baseURL }) => {
  const r = await request.get(new URL('version.json', baseURL).toString());
  expect(r.ok()).toBeTruthy();
  const v = await r.json();
  expect(v.version).toMatch(/^\d+\.\d+\.\d+$/);
  expect(v.commit).toMatch(/^[0-9a-f]{40}$/);
});
