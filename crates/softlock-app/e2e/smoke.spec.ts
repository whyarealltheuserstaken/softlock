import { expect, test, type Browser } from "@playwright/test";
import devServer from "../dev-server.json" with { type: "json" };
import platform from "../src/strings/platform.json" with { type: "json" };
import product from "../src/strings/product-name.json" with { type: "json" };

const origin = `http://${devServer.host}:${devServer.port}`;

test("window title comes from the product name", async ({ page }) => {
  await page.goto("/");
  await expect(page).toHaveTitle(product.productName);
  await expect(page.locator("main")).toHaveAttribute("data-ipc-bridge", "fake");
  await expect(page.locator("main")).toHaveAttribute("data-platform-support", "ready");
  await expect(page.getByRole("status")).toHaveCount(0);
});

test("macOS shows the unsupported platform message", async ({ browser }) => {
  await expectUnsupported(browser);
});

test("Linux shows the unsupported platform message", async ({ browser }) => {
  await expectUnsupported(browser);
});

async function expectUnsupported(browser: Browser) {
  const context = await browser.newContext({ baseURL: origin });
  await context.addInitScript((message) => {
    window.__SOFTLOCK_PLATFORM_STATUS__ = { supported: false, message };
  }, platform.unsupported);
  const page = await context.newPage();
  await page.goto("/");
  await expect(page.locator("main")).toHaveAttribute("data-platform-support", "unsupported");
  await expect(page.getByRole("status")).toHaveText(platform.unsupported);
  await context.close();
}
