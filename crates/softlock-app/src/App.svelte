<script lang="ts">
  import { onMount } from "svelte";
  import {
    createIpcBridge,
    loadPlatformStatus,
    pinErrorText,
    serviceLine,
    statusLine,
    type OwnerView,
  } from "./ipc/bridge";
  import ui from "./strings/ui.json";

  const bridge = createIpcBridge();
  let supported = true;
  let platformMessage: string | null = null;
  let view: OwnerView = bridge.current();
  let pin = "";

  onMount(() => {
    void loadPlatformStatus(bridge.kind).then((status) => {
      supported = status.supported;
      platformMessage = status.supported ? null : status.message;
    });
    return bridge.subscribe((next) => {
      view = next;
    });
  });

  async function lockFromApp() {
    view = await bridge.lock("App");
  }

  async function lockFromTray() {
    view = await bridge.lock("Tray");
  }

  async function lockForReal() {
    view = await bridge.hardLock();
  }

  async function savePin() {
    view = await bridge.setPin(pin);
  }

  function quit() {
    view = bridge.quit();
  }
</script>

<main
  data-ipc-bridge={bridge.kind}
  data-platform-support={supported ? "ready" : "unsupported"}
  data-lock-source={view.lockSource ?? ""}
  data-sources={view.sources}
  data-set-pin={view.setPinCount}
  data-quit={view.quit ? "local" : "no"}
  data-service-stop={view.serviceStop ? "true" : "false"}
  data-locked={view.locked ? "on" : "off"}
  data-notice={view.notice}
  data-hard-lock={view.hardLockCount}
>
  {#if platformMessage}
    <p class="platform-status" role="status">{platformMessage}</p>
  {/if}

  <p class="status-line" data-testid="status-line">{statusLine(view)}</p>
  <p class="service-line" data-testid="service-line">{serviceLine(view)}</p>
  {#if view.notice}
    <p data-testid="hotkey-notice">{view.notice}</p>
  {/if}

  {#if bridge.kind === "fake"}
    <section data-testid="tray">
      <p data-testid="tray-status">{statusLine(view)}</p>
      <p data-testid="tray-service">{serviceLine(view)}</p>
      <button type="button" data-testid="tray-lock" onclick={lockFromTray}>{ui.lockNow}</button>
      <button type="button" data-testid="tray-hard-lock" onclick={lockForReal}>{ui.lockForReal}</button>
      <button type="button" data-testid="tray-quit" onclick={quit}>{ui.quit}</button>
    </section>
  {/if}

  <form
    onsubmit={(event) => {
      event.preventDefault();
      void savePin();
    }}
  >
    <label>
      {ui.pinLabel}
      <input data-testid="pin" type="password" bind:value={pin} autocomplete="off" />
    </label>
    {#if pinErrorText(view.fieldError)}
      <p data-testid="pin-error">{pinErrorText(view.fieldError)}</p>
    {/if}
    <button type="submit" data-testid="set-pin">{ui.setPin}</button>
  </form>

  <button type="button" data-testid="lock" onclick={lockFromApp}>{ui.lock}</button>
</main>

<style>
  main {
    margin: 0;
    min-height: 100vh;
    color: CanvasText;
    background: Canvas;
    font-family: "Segoe UI Variable", "Segoe UI", sans-serif;
  }

  .platform-status,
  .status-line,
  .service-line {
    margin: 0;
    padding: 1.5rem 1.5rem 0;
    font-size: 1.125rem;
  }

  form,
  button,
  section {
    margin: 1rem 1.5rem;
  }

  label {
    display: block;
    margin-bottom: 0.5rem;
  }

  input {
    display: block;
    margin-top: 0.25rem;
    font: inherit;
  }
</style>
