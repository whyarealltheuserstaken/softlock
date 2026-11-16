<script lang="ts">
  import { onMount } from "svelte";
  import { createIpcBridge, loadPlatformStatus } from "./ipc/bridge";

  const bridge = createIpcBridge();
  let supported = true;
  let platformMessage: string | null = null;

  onMount(async () => {
    const status = await loadPlatformStatus(bridge.kind);
    supported = status.supported;
    platformMessage = status.supported ? null : status.message;
  });
</script>

<main
  data-ipc-bridge={bridge.kind}
  data-platform-support={supported ? "ready" : "unsupported"}
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
