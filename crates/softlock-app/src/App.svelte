<script lang="ts">
  import { createIpcBridge } from "./ipc/bridge";
  import platform from "./strings/platform.json" with { type: "json" };

  const bridge = createIpcBridge();
  const platformMessage = bridge.supported ? null : platform.unsupported;
</script>

<main
  data-ipc-bridge={bridge.kind}
  data-platform-support={bridge.supported ? "ready" : "unsupported"}
>
  {#if platformMessage}
    <p class="platform-status" role="status">{platformMessage}</p>
  {/if}
</main>

<style>
  main {
    margin: 0;
    min-height: 100vh;
    color: CanvasText;
    background: Canvas;
    font-family: "Segoe UI Variable", "Segoe UI", sans-serif;
  }

  .platform-status {
    margin: 0;
    padding: 1.5rem;
    font-size: 1.125rem;
  }
</style>
