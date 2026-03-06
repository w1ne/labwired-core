import { expect, test } from '@playwright/test';

test('landing to catalog navigation works', async ({ page }) => {
  await page.goto('/');
  await expect(page.getByText('Formally proven hardware')).toBeVisible();

  await page.getByRole('button', { name: 'Catalog', exact: true }).click();
  await expect(page).toHaveURL(/#\/catalog$/);
  await expect(page.getByText('VERIFIED ASSETS')).toBeVisible();
});

test('sidebar health button routes without dead click', async ({ page }) => {
  await page.goto('/#/catalog');
  await expect(page.getByText('VERIFIED ASSETS')).toBeVisible();

  await page.getByText('PLATFORM HEALTH').click();
  await expect(page).toHaveURL(/#\/health$/);
  // Health is auth-protected; signed-out users return to landing instead of dead route.
  await expect(page.getByText('Formally proven hardware')).toBeVisible();
});

test('catalog details button is clickable when assets exist', async ({ page }) => {
  await page.goto('/#/catalog');
  await expect(page.getByText('VERIFIED ASSETS')).toBeVisible();

  const detailsButtons = page.getByRole('button', { name: 'DETAILS' });
  if (await detailsButtons.count() === 0) {
    test.skip(true, 'No catalog assets available in smoke environment');
  }

  await detailsButtons.first().click();
  await expect(page.getByRole('button', { name: '← Back to catalog' })).toBeVisible();
});
