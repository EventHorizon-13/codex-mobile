import { mkdir } from 'node:fs/promises';
import { chromium } from '@playwright/test';

const baseURL = process.env.BASE_URL || 'http://127.0.0.1:3232';
const outputDir = process.env.REFERENCE_DIR || 'reference';

async function waitForConnection(page) {
  await page.goto(baseURL, { waitUntil: 'domcontentloaded' });
  await page.waitForFunction(
    () => document.querySelector('#state-label')?.textContent?.trim() !== 'offline',
    undefined,
    { timeout: 15_000 },
  );
}

async function captureScheme(browser, scheme) {
  const context = await browser.newContext({
    viewport: { width: 1180, height: 760 },
    deviceScaleFactor: 1,
    colorScheme: scheme,
    isMobile: false,
  });

  const page = await context.newPage();
  await waitForConnection(page);
  await page.locator('#msg-input').fill('TURN_GROUP_FIXTURE');
  await page.locator('#send-btn').waitFor({ state: 'visible' });
  await page.locator('#send-btn').click();
  await page.locator('.msg.codex').last().waitFor({ state: 'visible' });
  await page.waitForTimeout(1800);
  await page.screenshot({ path: `${outputDir}/web-${scheme}-main.png` });

  await page.locator('#menu-btn').click();
  await page.locator('#drawer.open').waitFor({ state: 'visible' });
  await page.waitForTimeout(300);
  await page.screenshot({ path: `${outputDir}/web-${scheme}-drawer-page.png` });
  await page.locator('#drawer').screenshot({ path: `${outputDir}/web-${scheme}-drawer.png` });
  await page.locator('#drawer-close').click();

  const emptyContext = await browser.newContext({
    viewport: { width: 1180, height: 760 },
    deviceScaleFactor: 1,
    colorScheme: scheme,
    isMobile: false,
  });
  const empty = await emptyContext.newPage();
  await waitForConnection(empty);
  await empty.screenshot({ path: `${outputDir}/web-${scheme}-empty.png` });
  await emptyContext.close();
  await context.close();
}

await mkdir(outputDir, { recursive: true });
const browser = await chromium.launch({ headless: true });
try {
  await captureScheme(browser, 'dark');
  await captureScheme(browser, 'light');
} finally {
  await browser.close();
}
