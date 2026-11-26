<script lang="ts">
  import CartSheet from '$lib/components/CartSheet.svelte';
  import { formatMoney } from '$lib/money';
  import { cart } from '$lib/stores/cart.svelte';
  import { session } from '$lib/stores/session.svelte';
  import type { Product } from '$lib/types';

  let search = $state('');
  let barcode = $state('');
  let showSearch = $state(false);
  let expanded = $state(false);
  let message = $state('');

  const products = $derived(session.catalog?.products ?? []);
  const currency = $derived(session.catalog?.currency ?? 'UGX');
  const filtered = $derived(
    products.filter((product) => {
      const query = search.trim().toLowerCase();
      if (!query) return true;
      return (
        product.name.toLowerCase().includes(query) ||
        (product.barcode ?? '').includes(query) ||
        (product.sku ?? '').toLowerCase().includes(query)
      );
    })
  );

  function add(product: Product): void {
    cart.add(product);
    message = `${product.name} added`;
  }

  function addBarcode(): void {
    const code = barcode.trim();
    const product = products.find((entry) => entry.barcode === code || entry.sku === code);
    if (!product) {
      message = `No product found for ${code || '(empty)'}`;
      return;
    }
    add(product);
    barcode = '';
  }

  function simulateScan(): void {
    const product = products[Math.floor(Math.random() * products.length)];
    if (product) add(product);
  }
</script>

<svelte:head>
  <title>Sell · Stride Companion</title>
</svelte:head>

<div class="sell">
  <header class="topbar">
    <span class="page-title">Sell</span>
    <button class="btn small" onclick={() => (showSearch = !showSearch)}>
      {showSearch ? 'Close' : 'Search'}
    </button>
  </header>

  {#if showSearch}
    <section class="search-panel">
      <input class="field" bind:value={search} placeholder="Name, barcode or SKU" aria-label="Search products" />
      <div class="categories">
        <span class="chip active">All ({products.length})</span>
        <span class="chip">Food</span>
        <span class="chip">Drinks</span>
        <span class="chip">Household</span>
      </div>
      <div class="results">
        {#each filtered as product (product.id)}
          <button class="result" onclick={() => add(product)}>
            <span>
              <strong>{product.name}</strong>
              <small class="muted">{product.sku ?? product.barcode ?? ''}</small>
            </span>
            <span class="money">{formatMoney(product.price_minor, currency)}</span>
          </button>
        {:else}
          <p class="muted">No matching products in this catalog snapshot.</p>
        {/each}
      </div>
      <p class="hint muted">Unknown barcode? It can be entered manually on the main till.</p>
    </section>
  {:else}
    <section class="camera" aria-label="Barcode camera preview">
      <div class="frame">
        <span class="corner tl"></span>
        <span class="corner tr"></span>
        <span class="corner bl"></span>
        <span class="corner br"></span>
        <span class="laser"></span>
      </div>
      <p class="camera-hint muted">Point at a barcode · camera preview placeholder in browser</p>
    </section>
  {/if}

  <div class="manual">
    <form onsubmit={(event) => { event.preventDefault(); addBarcode(); }}>
      <input
        class="field"
        bind:value={barcode}
        inputmode="numeric"
        placeholder="Or type a barcode and press Enter"
        aria-label="Barcode"
      />
    </form>
    <button class="btn small" onclick={simulateScan}>Simulate scan</button>
  </div>

  {#if message}
    <p class="message" role="status">{message}</p>
  {/if}

  <CartSheet bind:expanded />
</div>

<style>
  .sell {
    display: flex;
    flex-direction: column;
    min-height: calc(100vh - var(--status-height));
  }

  .topbar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: var(--space-md) var(--space-lg);
    background: var(--surface);
    border-bottom: 1px solid var(--border);
  }

  .btn.small {
    min-height: var(--touch);
    padding: var(--space-xs) var(--space-md);
  }

  .camera {
    position: relative;
    flex: 1;
    min-height: 34vh;
    display: grid;
    place-items: center;
    background: var(--bg);
    overflow: hidden;
  }

  .frame {
    position: relative;
    width: 68%;
    max-width: 280px;
    aspect-ratio: 3 / 2;
    border: 1px solid rgb(255 255 255 / 40%);
    border-radius: var(--radius-md);
    background: rgb(0 0 0 / 6%);
  }

  .corner {
    position: absolute;
    width: 28px;
    height: 28px;
    border: 3px solid var(--accent);
  }

  .tl { top: -2px; left: -2px; border-right: 0; border-bottom: 0; border-radius: var(--radius-md) 0 0 0; }
  .tr { top: -2px; right: -2px; border-left: 0; border-bottom: 0; border-radius: 0 var(--radius-md) 0 0; }
  .bl { bottom: -2px; left: -2px; border-right: 0; border-top: 0; border-radius: 0 0 0 var(--radius-md); }
  .br { bottom: -2px; right: -2px; border-left: 0; border-top: 0; border-radius: 0 0 var(--radius-md) 0; }

  .laser {
    position: absolute;
    left: 8%;
    right: 8%;
    top: 50%;
    height: 2px;
    background: var(--accent);
    opacity: 0.6;
  }

  .camera-hint {
    position: absolute;
    bottom: var(--space-sm);
    font-size: var(--type-small);
  }

  .search-panel {
    flex: 1;
    min-height: 34vh;
    padding: var(--space-md) var(--space-lg);
    background: var(--surface);
    overflow-y: auto;
  }

  .categories {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-xs);
    margin: var(--space-sm) 0;
  }

  .chip.active {
    background: var(--accent-soft);
    color: var(--accent);
  }

  .result {
    width: 100%;
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-md);
    min-height: var(--touch);
    padding: var(--space-sm) 0;
    border: 0;
    border-bottom: 1px solid var(--border);
    background: transparent;
    text-align: left;
  }

  .result small {
    display: block;
    font-size: var(--type-small);
  }

  .hint {
    margin-top: var(--space-md);
    font-size: var(--type-small);
  }

  .manual {
    display: flex;
    gap: var(--space-sm);
    padding: var(--space-sm) var(--space-lg);
    background: var(--surface);
    border-top: 1px solid var(--border);
  }

  .message {
    padding: var(--space-xs) var(--space-lg);
    background: var(--accent-soft);
    font-size: var(--type-small);
  }

  @media (min-width: 600px) {
    .sell {
      display: block;
    }

    .camera,
    .search-panel {
      min-height: 40vh;
    }
  }
</style>
