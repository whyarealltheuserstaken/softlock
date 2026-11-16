import { expect, test } from "@playwright/test";
import product from "../src/strings/product-name.json" with { type: "json" };

test("window title comes from the product name", async ({ page }) => {
  await page.goto("/");
  await expect(page).toHaveTitle(product.productName);
  await expect(page.locator("main")).toHaveAttribute("data-ipc-bridge", "fake");
});
