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

test("tray lock now and the app button record tray and app sources", async ({ page }) => {
  await page.goto("/");
  await page.getByTestId("tray-lock").click();
  await expect(page.locator("main")).toHaveAttribute("data-lock-source", "Tray");
  await page.getByTestId("lock").click();
  await expect(page.locator("main")).toHaveAttribute("data-lock-source", "App");
  await expect(page.locator("main")).toHaveAttribute("data-sources", "Tray,App");
  await expect(page.locator("main")).toHaveAttribute("data-locked", "on");
});

test("subscribe shows locked and returns to unlocked after the pin stream", async ({ page }) => {
  await page.goto("/");
  await expect(page.getByTestId("status-line")).toContainText("is off");
  await expect(page.getByTestId("tray-status")).toContainText("is off");
  await page.evaluate(() => window.__SOFTLOCK_PUSH_STATE__?.("Locked"));
  await expect(page.getByTestId("status-line")).toContainText("is on");
  await expect(page.getByTestId("tray-status")).toContainText("is on");
  await expect(page.locator("main")).toHaveAttribute("data-locked", "on");
  await page.evaluate(() => window.__SOFTLOCK_PUSH_STATE__?.("Unlocked"));
  await expect(page.getByTestId("status-line")).toContainText("is off");
  await expect(page.getByTestId("tray-status")).toContainText("is off");
  await expect(page.locator("main")).toHaveAttribute("data-locked", "off");
});

test("a pipe error does not show the unlocked state", async ({ page }) => {
  await page.goto("/");
  await page.evaluate(() => window.__SOFTLOCK_PUSH_STATE__?.("Unlocked"));
  await page.evaluate(() => window.__SOFTLOCK_FAIL_PIPE__?.());
  await expect(page.getByTestId("status-line")).toContainText("is on");
  await expect(page.getByTestId("service-line")).toHaveText("Service not running");
  await expect(page.locator("main")).toHaveAttribute("data-locked", "on");
});

test("settings rejects a too-short pin and sets a long enough pin", async ({ page }) => {
  await page.goto("/");
  await page.getByTestId("pin").fill("abc");
  await page.getByTestId("set-pin").click();
  await expect(page.getByTestId("pin-error")).toHaveText("PIN is too short");
  await expect(page.locator("main")).toHaveAttribute("data-set-pin", "0");
  await page.getByTestId("pin").fill("1357");
  await page.getByTestId("set-pin").click();
  await expect(page.getByTestId("pin-error")).toHaveCount(0);
  await expect(page.locator("main")).toHaveAttribute("data-set-pin", "1");
});

test("a missing hotkey is shown in the window", async ({ page }) => {
  await page.goto("/");
  await page.evaluate(() => window.__SOFTLOCK_SET_NOTICE__?.("hotkey unavailable"));
  await expect(page.getByTestId("hotkey-notice")).toHaveText("hotkey unavailable");
  await expect(page.locator("main")).toHaveAttribute("data-notice", "hotkey unavailable");
});

test("a pin that needs confirmation is not treated as a lock failure", async ({ page }) => {
  await page.goto("/");
  await page.evaluate(() => {
    window.__SOFTLOCK_PIN_RESULT__ = "needs-confirmation";
  });
  await page.getByTestId("pin").fill("1357");
  await page.getByTestId("set-pin").click();
  await expect(page.getByTestId("pin-error")).toHaveText("Confirmation needed");
  await expect(page.locator("main")).toHaveAttribute("data-locked", "off");
  await expect(page.locator("main")).toHaveAttribute("data-set-pin", "0");
  await expect(page.getByTestId("service-line")).toHaveText("Service running");
});

test("lock for real is a separate tray action", async ({ page }) => {
  await page.goto("/");
  await page.getByTestId("tray-hard-lock").click();
  await expect(page.locator("main")).toHaveAttribute("data-hard-lock", "1");
  await expect(page.locator("main")).toHaveAttribute("data-lock-source", "");
  await expect(page.getByTestId("tray-hard-lock")).toHaveText("Lock for real");
});

test("a fall-back event shows the time and the reason after sign-in", async ({ page }) => {
  await page.goto("/");
  await page.evaluate(() =>
    window.__SOFTLOCK_FALLBACK__?.(1500, "the lock host stopped answering"),
  );
  await expect(page.getByTestId("hotkey-notice")).toContainText(
    "switched to the Windows lock at 1500 ms because the lock host stopped answering",
  );
  await expect(page.locator("main")).toHaveAttribute("data-locked", "off");
});

test("quitting the app does not stop the service", async ({ page }) => {
  await page.goto("/");
  await page.getByTestId("tray-quit").click();
  await expect(page.locator("main")).toHaveAttribute("data-quit", "local");
  await expect(page.locator("main")).toHaveAttribute("data-service-stop", "false");
  await expect(page.getByTestId("service-line")).toHaveText("Service running");
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
