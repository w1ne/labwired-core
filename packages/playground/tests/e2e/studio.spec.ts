import { test, expect, type Page } from '@playwright/test';

const gotoPlayground = (page: Page) =>
  page.goto('/', { waitUntil: 'domcontentloaded' });

test('top chrome renders with LabWired brand', async ({ page }) => {
  await gotoPlayground(page);
  await expect(page.getByRole('link', { name: /labwired/i })).toBeVisible();
});

test('Code toggle hides and reveals the dev drawer', async ({ page }) => {
  await gotoPlayground(page);
  const serialTab = page.getByRole('tab', { name: /serial/i });
  await expect(serialTab).toBeVisible();

  await page.getByRole('switch', { name: /hide code editor/i }).click();
  await expect(serialTab).toBeHidden();

  await page.getByRole('switch', { name: /show code editor/i }).click();
  await expect(serialTab).toBeVisible();
});

test('⌘K opens the command palette', async ({ page }) => {
  await gotoPlayground(page);
  await page.keyboard.press('Meta+K');
  // If Meta+K doesn't fire on Linux CI, try Control+K
  const dialog = page.getByRole('dialog', { name: /command palette/i });
  await expect(dialog).toBeVisible({ timeout: 2000 }).catch(async () => {
    await page.keyboard.press('Control+K');
    await expect(dialog).toBeVisible({ timeout: 2000 });
  });
});

test('default starter circuit renders on the canvas', async ({ page }) => {
  await gotoPlayground(page);
  await expect(page.getByRole('main', { name: /canvas/i })).toContainText('STM32F103');
  await expect(page.getByRole('toolbar', { name: /simulation controls/i })).toBeVisible();
});
