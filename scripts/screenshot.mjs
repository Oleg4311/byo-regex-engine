// Снимок песочницы для README: npm run site, затем node scripts/serve.mjs и node scripts/screenshot.mjs
import { chromium } from '@playwright/test';

const browser = await chromium.launch();
const page = await browser.newPage({ viewport: { width: 1100, height: 900 } });
await page.goto('http://127.0.0.1:4183/');
await page.locator('#hl mark').first().waitFor();
await page.locator('#n').fill('26');
await page.locator('#boom').click();
await page.locator('#chart').click();
await page.locator('#svg polyline').first().waitFor();
await page.screenshot({ path: 'docs/screenshot.png', fullPage: true });
await browser.close();
