<script lang="ts">
  import { goto } from '$app/navigation';
  import { formatMoney } from '$lib/money';
  import { cart } from '$lib/stores/cart.svelte';
  import { session } from '$lib/stores/session.svelte';

  let { expanded = $bindable(false) }: { expanded?: boolean } = $props();

  const currency = $derived(session.catalog?.currency ?? 'UGX');
</script>

<section class="sheet" class:expanded aria-label="Cart">
  <button class="handle" onclick={() => (expanded = !expanded)} aria-expanded={expanded}>
    <span class="grip"></span>
    <span>CART · {cart.count} {cart.count === 1 ? 'item' : 'items'}</span>
    <span class="chevron">{expanded ? '⌄' : '⌃'}</span>
  </button>

  {#if expanded}
    <div class="lines">
      {#each cart.lines as line (line.product.id)}
        <div class="line">
          <div class="name">
            <strong>{line.product.name}</strong>
            <span class="muted">{formatMoney(line.unit_minor, currency)} each</span>
          </div>
          <div class="qty">
            <button onclick={() => cart.setQty(line.product.id, line.qty - 1)} aria-label="Decrease quantity">−</button>
            <span>{line.qty}</span>
            <button onclick={() => cart.setQty(line.product.id, line.qty + 1)} aria-label="Increase quantity">+</button>
          </div>
          <span class="money total">{formatMoney(line.total_minor, currency)}</span>
        </div>
      {:else}
        <p class="muted empty">No items yet. Scan a barcode or search above.</p>
      {/each}
    </div>
  {/if}

  <div class="footer">
    <div>
      <span class="eyebrow">TOTAL</span>
      <strong class="money amount">{formatMoney(cart.subtotal_minor, currency)}</strong>
    </div>
    <button
      class="btn btn-primary"
      disabled={cart.count === 0}
      onclick={() => goto('/checkout')}
    >
      Take payment
    </button>
  </div>
</section>

<style>
  .sheet {
    background: var(--surface);
    border-top: 1px solid var(--border);
    border-radius: var(--radius-md) var(--radius-md) 0 0;
    box-shadow: 0 -4px 16px rgb(0 0 0 / 8%);
  }

  .handle {
    width: 100%;
    min-height: 36px;
    display: flex;
    align-items: center;
    gap: var(--space-sm);
    padding: var(--space-xs) var(--space-lg);
    border: 0;
    background: transparent;
    color: var(--text-muted);
    font-size: var(--type-small);
    font-weight: 600;
    letter-spacing: 0.04em;
  }

  .grip {
    width: 32px;
    height: 4px;
    border-radius: var(--radius-pill);
    background: var(--border-strong);
  }

  .chevron {
    margin-left: auto;
  }

  .lines {
    max-height: 34vh;
    overflow-y: auto;
    padding: 0 var(--space-lg) var(--space-sm);
  }

  .line {
    display: grid;
    grid-template-columns: 1fr auto auto;
    align-items: center;
    gap: var(--space-sm);
    padding: var(--space-sm) 0;
    border-bottom: 1px solid var(--border);
  }

  .name {
    display: flex;
    flex-direction: column;
    min-width: 0;
  }

  .name strong {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .name span {
    font-size: var(--type-small);
  }

  .qty {
    display: flex;
    align-items: center;
    gap: var(--space-xs);
  }

  .qty button {
    width: 32px;
    height: 32px;
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: var(--surface-alt);
  }

  .total {
    font-size: var(--type-body);
  }

  .empty {
    padding: var(--space-md) 0;
  }

  .footer {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-md);
    padding: var(--space-sm) var(--space-lg)
      calc(var(--space-sm) + var(--nav-row) + env(safe-area-inset-bottom, 0));
  }

  .amount {
    display: block;
    font-size: var(--type-total);
    line-height: 1.1;
  }
</style>
