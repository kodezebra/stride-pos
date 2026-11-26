<script lang="ts">
  import { goto } from '$app/navigation';
  import { discountMinor, formatMoney, roundToStep } from '$lib/money';
  import { cart } from '$lib/stores/cart.svelte';
  import { session } from '$lib/stores/session.svelte';
  import type { Sale } from '$lib/types';

  let tab = $state<'cash' | 'mobile'>('cash');
  let tendered = $state('');
  let reference = $state('');
  let network = $state<'mtn_momo' | 'airtel_money'>('mtn_momo');
  let useCredit = $state(false);
  let customerId = $state('');
  let error = $state('');

  const currency = $derived(session.catalog?.currency ?? 'UGX');
  const step = $derived(session.catalog?.cash_step ?? 100);
  const customers = $derived(session.catalog?.customers ?? []);
  const customer = $derived(customers.find((entry) => entry.id === customerId));
  const discount = $derived(
    useCredit && customer ? discountMinor(cart.subtotal_minor, customer.discount_bp) : 0
  );
  const payable = $derived(roundToStep(cart.subtotal_minor - discount, step));
  const tenderedMinor = $derived(Number(tendered) || 0);
  const change = $derived(Math.max(0, tenderedMinor - payable));
  const mobileCap = $derived(session.catalog?.caps.max_mobile_money_minor ?? 0);
  const overCap = $derived(payable > mobileCap);
  const creditLimit = $derived(customer?.credit_limit_minor ?? 0);
  const overCredit = $derived(useCredit && payable > creditLimit);

  function press(key: string): void {
    error = '';
    if (key === '⌫') {
      tendered = tendered.slice(0, -1);
      return;
    }
    if (key === 'C') {
      tendered = '';
      return;
    }
    tendered = `${tendered}${key}`.replace(/^0+/, '') || '0';
  }

  async function finish(tender: Sale['tender']): Promise<void> {
    error = '';
    if (cart.count === 0) {
      error = 'The cart is empty.';
      return;
    }
    if (tender === 'cash' && tenderedMinor < payable) {
      error = 'Tendered amount is below the total.';
      return;
    }
    if ((tender === 'mtn_momo' || tender === 'airtel_money') && !reference.trim()) {
      error = 'Enter the payment reference.';
      return;
    }
    if ((tender === 'mtn_momo' || tender === 'airtel_money') && overCap) {
      error = 'Above this device’s mobile money limit. Use the main till.';
      return;
    }
    if (tender === 'credit' && !customer) {
      error = 'Choose a customer for the credit sale.';
      return;
    }
    if (tender === 'credit' && overCredit) {
      error = 'Above this customer’s credit limit. Use the main till.';
      return;
    }

    await session.sell({
      total_minor: payable,
      tender,
      reference: tender === 'cash' || tender === 'credit' ? undefined : reference.trim(),
      customer_id: tender === 'credit' ? customerId : undefined,
      lines: cart.lines.map((line) => ({
        name: line.product.name,
        qty: line.qty,
        unit_minor: line.unit_minor,
        product_id: line.product.id
      }))
    });
    cart.clear();
    await goto('/queue');
  }

  async function confirm(): Promise<void> {
    const tender: Sale['tender'] = useCredit
      ? 'credit'
      : tab === 'cash'
        ? 'cash'
        : network;
    await finish(tender);
  }
</script>

<svelte:head>
  <title>Checkout · Stride Companion</title>
</svelte:head>

<div class="page checkout">
  <header class="topbar">
    <button class="btn small" onclick={() => goto('/sell')}>← Sale</button>
    <span class="page-title">Checkout</span>
    <span class="muted small-text">{session.cashier}</span>
  </header>

  <section class="card total-card">
    <span class="eyebrow">Total payable</span>
    <strong class="money amount">{formatMoney(payable, currency)}</strong>
    <span class="muted small-text">
      items {cart.count} · discount −{formatMoney(discount, currency)} · tax incl. {formatMoney(0, currency)}
    </span>
  </section>

  <div class="tabs" role="tablist" aria-label="Tender type">
    <button role="tab" aria-selected={tab === 'cash'} class:active={tab === 'cash'} onclick={() => { tab = 'cash'; error = ''; }}>
      Cash
    </button>
    <button role="tab" aria-selected={tab === 'mobile'} class:active={tab === 'mobile'} onclick={() => { tab = 'mobile'; error = ''; }}>
      Mobile money
    </button>
  </div>

  {#if tab === 'cash'}
    <section class="card">
      <div class="readout">
        <span class="eyebrow">Tendered</span>
        <strong class="money">{formatMoney(tenderedMinor, currency)}</strong>
      </div>
      <div class="readout change">
        <span class="eyebrow">Change</span>
        <strong class="money">{formatMoney(change, currency)}</strong>
      </div>
      <div class="keypad">
        {#each ['1', '2', '3', '4', '5', '6', '7', '8', '9', 'C', '0', '⌫'] as key (key)}
          <button onclick={() => press(key)}>{key}</button>
        {/each}
      </div>
    </section>
  {:else}
    <section class="card">
      <div class="networks">
        <label>
          <input type="radio" bind:group={network} value="mtn_momo" />
          MTN MoMo
        </label>
        <label>
          <input type="radio" bind:group={network} value="airtel_money" />
          Airtel Money
        </label>
      </div>
      <label class="stack">
        <span class="eyebrow">Reference</span>
        <input class="field" bind:value={reference} placeholder="Transaction code" />
      </label>
      <p class="muted small-text">
        Recorded, not verified · limit {formatMoney(mobileCap, currency)}
      </p>
      {#if overCap}
        <p class="limit">Above this device’s limit? Use the main till.</p>
      {/if}
    </section>
  {/if}

  <label class="card credit-toggle">
    <input type="checkbox" bind:checked={useCredit} />
    <span>Put on customer account</span>
  </label>

  {#if useCredit}
    <section class="card">
      <select class="field" bind:value={customerId} aria-label="Customer">
        <option value="">Choose a customer…</option>
        {#each customers as entry (entry.id)}
          <option value={entry.id}>{entry.name}</option>
        {/each}
      </select>
      <p class="muted small-text">
        Available credit {formatMoney(creditLimit, currency)}
        {#if customer} · discount {customer.discount_bp / 100}%{/if}
      </p>
      {#if overCredit}
        <p class="limit">Above this customer’s credit limit. Use the main till.</p>
      {/if}
    </section>
  {/if}

  {#if error}
    <p class="error" role="alert">{error}</p>
  {/if}

  <button class="btn btn-primary wide" onclick={confirm}>Confirm sale</button>
  <p class="muted small-text footnote">No approvals on this device. Limits come from the till.</p>
</div>

<style>
  .topbar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-sm);
  }

  .btn.small {
    min-height: var(--touch);
    padding: var(--space-xs) var(--space-md);
  }

  .small-text {
    font-size: var(--type-small);
  }

  .total-card {
    margin-top: var(--space-lg);
    display: flex;
    flex-direction: column;
    gap: var(--space-xs);
  }

  .amount {
    font-size: var(--type-total);
    line-height: 1.1;
  }

  .tabs {
    display: flex;
    margin: var(--space-lg) 0 0;
    border-bottom: 1px solid var(--border);
  }

  .tabs button {
    flex: 1;
    min-height: var(--touch);
    border: 0;
    border-bottom: 3px solid transparent;
    background: transparent;
    color: var(--text-muted);
    font-weight: 600;
  }

  .tabs button.active {
    border-bottom-color: var(--accent);
    color: var(--accent);
    background: var(--accent-soft);
  }

  .readout {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    padding: var(--space-sm) 0;
  }

  .readout.change strong {
    color: var(--success);
  }

  .keypad {
    display: grid;
    grid-template-columns: repeat(3, 1fr);
    gap: var(--space-xs);
    margin-top: var(--space-md);
  }

  .keypad button {
    min-height: 52px;
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: var(--surface-alt);
    font-family: var(--font-numeric);
    font-size: 18px;
  }

  .networks {
    display: flex;
    gap: var(--space-lg);
    margin-bottom: var(--space-md);
  }

  .networks label,
  .credit-toggle {
    display: flex;
    align-items: center;
    gap: var(--space-sm);
  }

  .credit-toggle {
    margin-top: var(--space-lg);
  }

  .stack {
    display: flex;
    flex-direction: column;
    gap: var(--space-xs);
  }

  .wide {
    width: 100%;
    margin-top: var(--space-lg);
  }

  .limit {
    margin-top: var(--space-sm);
    color: var(--warning);
    font-size: var(--type-small);
  }

  .error {
    margin-top: var(--space-md);
    padding: var(--space-sm);
    border-radius: var(--radius-sm);
    background: var(--surface-alt);
    border: 1px solid var(--danger);
    color: var(--danger);
    font-size: var(--type-small);
  }

  .footnote {
    margin-top: var(--space-md);
    text-align: center;
  }
</style>
