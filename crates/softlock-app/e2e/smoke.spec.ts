import { expect, test } from "@playwright/test";

test("window title comes from the product name", async ({ page }) => {
  await page.goto("/");
  await expect(page).toHaveTitle("SoftLock");
  await expect(page.locator("main")).toHaveAttribute("data-ipc-bridge", "fake");
});
