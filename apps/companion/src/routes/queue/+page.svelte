<script lang="ts">
  import { goto } from '$app/navigation';
  import { formatMoney } from '$lib/money';
  import { session } from '$lib/stores/session.svelte';

  let busy = $state(false);
  let summary = $state<{ accepted: number; rejected: number } | null>(null);

  const currency = $derived(session.catalog?.currency ?? 'UGX');
  const pending = $derived(session.sales.filter((sale) => sale.status === 'pending').length);
  const synced = $derived(session.sales.filter((sale) => sale.status === 'synced').length);

  function time(value: string): string {
    return new Date(value).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });
  }

  async function sync(): Promise<void> {
    busy = true;
    summary = await session.sync();
    busy = false;
  }
</script>

<svelte:head>
  <title>Queue · Stride Companion</title>
</svelte:head>

<div class="page">
  <header class="topbar">
    <span class="page-title">Sales queue</span>
    <button class="btn small" onclick={() => goto('/sell')}>New sale</button>
  </header>

  {#if summary}
    <section class="card summary" role="status">
      <strong>Sync complete</strong>
      <span>{summary.accepted} accepted · {summary.rejected} rejected</span>
      <span class="muted small">Catalog updated to v{session.catalog?.version ?? '—'}</span>
    </section>
  {/if}

  <div class="counts">
    <div class="card count">
      <span class="eyebrow">Pending</span>
      <strong>{pending}</strong>
    </div>
    <div class="card count">
      <span class="eyebrow">Synced</span>
      <strong>{synced}</strong>
    </div>
  </div>

  <section class="card list">
    {#each session.sales as sale (sale.uid)}
      <div class="sale">
        <div class="left">
          <strong class="money amount">{formatMoney(sale.total_minor, currency)}</strong>
          <span class="muted small">
            {time(sale.occurred_at)} · {sale.tender.replace('_', ' ')} · {sale.cashier}
          </span>
          {#if sale.unverified}
            <span class="flag">Recorded, not verified</span>
          {/if}
        </div>
        <span class="status" class:synced={sale.status === 'synced'}>
          {sale.status === 'pending' ? '● Queued' : '✓ Synced'}
        </span>
      </div>
    {:else}
      <p class="muted">No sales yet. Start with a scan.</p>
    {/each}
  </section>

  <button class="btn btn-primary wide" onclick={sync} disabled={busy || pending === 0}>
    {busy ? 'Syncing…' : 'Sync now'}
  </button>
  <p class="muted small note">
    Closing this device is held while sales are pending, just like the main till holds a shift
    until its receipts are saved.
  </p>
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

  .summary {
    margin-top: var(--space-lg);
    display: flex;
    flex-direction: column;
    gap: var(--space-xs);
    background: var(--accent-soft);
    border-color: var(--accent);
  }

  .small {
    font-size: var(--type-small);
  }

  .counts {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: var(--space-md);
    margin-top: var(--space-lg);
  }

  .count {
    display: flex;
    flex-direction: column;
    gap: var(--space-xs);
  }

  .count strong {
    font-size: var(--type-kpi);
  }

  .list {
    margin-top: var(--space-lg);
    padding: 0 var(--space-lg);
  }

  .sale {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-md);
    padding: var(--space-md) 0;
    border-bottom: 1px solid var(--border);
  }

  .sale:last-child {
    border-bottom: 0;
  }

  .left {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
  }

  .amount {
    font-size: var(--type-heading);
  }

  .flag {
    color: var(--warning);
    font-size: var(--type-small);
  }

  .status {
    white-space: nowrap;
    color: var(--accent);
    font-size: var(--type-small);
    font-weight: 600;
  }

  .status.synced {
    color: var(--success);
  }

  .wide {
    width: 100%;
    margin-top: var(--space-lg);
  }

  .note {
    margin-top: var(--space-md);
  }
</style>
