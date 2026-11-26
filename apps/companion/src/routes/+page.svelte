<script lang="ts">
  import { goto } from '$app/navigation';
  import { session } from '$lib/stores/session.svelte';
  import { formatMoney } from '$lib/money';
  import { cart } from '$lib/stores/cart.svelte';

  const lastSync = $derived(
    session.status?.last_sync
      ? new Date(session.status.last_sync).toLocaleTimeString([], {
          hour: '2-digit',
          minute: '2-digit'
        })
      : '—'
  );
</script>

<svelte:head>
  <title>Ready · Stride Companion</title>
</svelte:head>

<div class="page">
  <header class="topbar">
    <span class="eyebrow">Stride Companion</span>
    <span class="chip">⚡ {session.status?.battery ?? '--'}%</span>
  </header>

  <section class="card hero">
    <p class="eyebrow">Ready to sell</p>
    <h1 class="page-title">The till can go down.<br />You can keep selling.</h1>

    <div class="facts">
      <div>
        <span class="eyebrow">Catalog</span>
        <strong>v{session.catalog?.version ?? '—'} · {session.catalog?.products.length ?? 0} items</strong>
      </div>
      <div>
        <span class="eyebrow">Last sync</span>
        <strong>{lastSync}</strong>
      </div>
      <div>
        <span class="eyebrow">Cashier</span>
        <strong>{session.cashier || '—'}</strong>
      </div>
      <div>
        <span class="eyebrow">Device limit</span>
        <strong class="money">{formatMoney(session.catalog?.caps.max_sale_minor ?? 0, session.catalog?.currency)}</strong>
      </div>
    </div>

    <a class="btn btn-primary wide" href="/sell">Start selling</a>
  </section>

  <button class="card queue-link" onclick={() => goto('/queue')}>
    <div>
      <span class="eyebrow">Sales waiting to sync</span>
      <strong>{session.status?.pending ?? 0}</strong>
    </div>
    <span aria-hidden="true">→</span>
  </button>

  <p class="note muted">
    Sales are stored on this device and sent to the main till when power returns.
    {#if cart.count > 0}Your current cart has {cart.count} items.{/if}
  </p>
</div>

<style>
  .topbar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: var(--space-md) 0;
  }

  .hero {
    padding: var(--space-xl) var(--space-lg);
  }

  .page-title {
    margin-top: var(--space-sm);
    line-height: 1.25;
  }

  .facts {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: var(--space-lg);
    margin: var(--space-xl) 0;
  }

  .facts > div {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .wide {
    width: 100%;
  }

  .queue-link {
    width: 100%;
    display: flex;
    align-items: center;
    justify-content: space-between;
    text-align: left;
    margin-top: var(--space-lg);
  }

  .queue-link strong {
    font-size: var(--type-kpi);
  }

  .note {
    margin-top: var(--space-lg);
    font-size: var(--type-small);
  }

  @media (min-width: 600px) {
    .facts {
      grid-template-columns: repeat(4, 1fr);
    }
  }
</style>
