<script lang="ts">
  import { goto } from '$app/navigation';
  import { session } from '$lib/stores/session.svelte';

  let host = $state('192.168.1.10');
  let port = $state(47831);
  let token = $state('');
  let error = $state('');
  let busy = $state(false);

  async function connect(): Promise<void> {
    error = '';
    busy = true;
    try {
      await session.pair(host.trim(), Number(port), token.trim());
      await goto('/');
    } catch (cause) {
      error = cause instanceof Error ? cause.message : String(cause);
    } finally {
      busy = false;
    }
  }
</script>

<svelte:head>
  <title>Pair · Stride Companion</title>
</svelte:head>

<div class="page">
  <header class="topbar">
    <button class="btn small" onclick={() => goto('/')}>← Back</button>
    <span class="page-title">Connect to till</span>
  </header>

  <section class="card">
    <p class="hint muted">
      Scan the pairing QR shown in the till's Settings → Phone &amp; tablet screen, or type the
      host, port and code by hand.
    </p>

    <label class="stack">
      <span class="eyebrow">Host</span>
      <input class="field" bind:value={host} autocomplete="off" />
    </label>
    <label class="stack">
      <span class="eyebrow">Port</span>
      <input class="field" bind:value={port} type="number" />
    </label>
    <label class="stack">
      <span class="eyebrow">Pairing code</span>
      <input class="field" bind:value={token} placeholder="AB12 CD34 EF56" autocomplete="off" />
    </label>

    {#if error}
      <p class="error" role="alert">{error}</p>
    {/if}

    <button class="btn btn-primary wide" onclick={connect} disabled={busy || !token.trim()}>
      {busy ? 'Connecting…' : 'Test and connect'}
    </button>
  </section>
</div>

<style>
  .topbar {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }

  .btn.small {
    min-height: var(--touch);
    padding: var(--space-xs) var(--space-md);
  }

  .hint {
    font-size: var(--type-small);
    margin-bottom: var(--space-lg);
  }

  .stack {
    display: flex;
    flex-direction: column;
    gap: var(--space-xs);
    margin-bottom: var(--space-md);
  }

  .wide {
    width: 100%;
    margin-top: var(--space-lg);
  }

  .error {
    padding: var(--space-sm);
    border: 1px solid var(--danger);
    border-radius: var(--radius-sm);
    color: var(--danger);
    font-size: var(--type-small);
  }
</style>
