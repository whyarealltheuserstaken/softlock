import { expect, test, type Browser } from "@playwright/test";
import devServer from "../dev-server.json" with { type: "json" };
import platform from "../src/strings/platform.json" with { type: "json" };
import product from "../src/strings/product-name.json" with { type: "json" };

const origin = `http://${devServer.host}:${devServer.port}`;

const LINUX_UA =
  "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/122.0.0.0 Safari/537.36";
const MAC_UA =
  "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/122.0.0.0 Safari/537.36";

test("window title comes from the product name", async ({ page }) => {
  await page.goto("/");
  await expect(page).toHaveTitle(product.productName);
  await expect(page.locator("main")).toHaveAttribute("data-ipc-bridge", "fake");
  await expect(page.locator("main")).toHaveAttribute("data-platform-support", "ready");
  await expect(page.getByRole("status")).toHaveCount(0);
});

test("macOS shows the unsupported platform message", async ({ browser }) => {
  await expectUnsupported(browser, MAC_UA);
});

test("Linux shows the unsupported platform message", async ({ browser }) => {
  await expectUnsupported(browser, LINUX_UA);
});

async function expectUnsupported(browser: Browser, userAgent: string) {
  const context = await browser.newContext({ userAgent, baseURL: origin });
  const page = await context.newPage();
  await page.goto("/");
  await expect(page.locator("main")).toHaveAttribute("data-platform-support", "unsupported");
  await expect(page.getByRole("status")).toHaveText(platform.unsupported);
  await context.close();
}
