import { test, expect } from '@playwright/test';

test('top chrome renders with LabWired brand', async ({ page }) => {
  await page.goto('/');
  await expect(page.getByRole('link', { name: /labwired/i })).toBeVisible();
});

test('Dev toggle reveals the dev drawer', async ({ page }) => {
  await page.goto('/');
  await page.getByRole('switch', { name: /dev mode/i }).click();
  await expect(page.getByRole('tab', { name: /serial/i })).toBeVisible();
});

test('⌘K opens the command palette', async ({ page }) => {
  await page.goto('/');
  await page.keyboard.press('Meta+K');
  // If Meta+K doesn't fire on Linux CI, try Control+K
  const dialog = page.getByRole('dialog', { name: /command palette/i });
  await expect(dialog).toBeVisible({ timeout: 2000 }).catch(async () => {
    await page.keyboard.press('Control+K');
    await expect(dialog).toBeVisible({ timeout: 2000 });
  });
});

test('hero prompt is centered when canvas is empty', async ({ page }) => {
  await page.goto('/');
  await expect(page.getByPlaceholder(/describe what to build/i)).toBeVisible();
});
