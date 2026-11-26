<script lang="ts">
  import { page } from '$app/stores';
  import { session } from '$lib/stores/session.svelte';

  const items = [
    { href: '/', label: 'Ready', icon: '⌂' },
    { href: '/sell', label: 'Sell', icon: '▤' },
    { href: '/queue', label: 'Queue', icon: '⟳' }
  ];
</script>

<nav aria-label="Main">
  {#each items as item (item.href)}
    <a href={item.href} class:active={$page.url.pathname === item.href}>
      <span class="icon" aria-hidden="true">{item.icon}</span>
      <span>{item.label}</span>
      {#if item.href === '/queue' && (session.status?.pending ?? 0) > 0}
        <span class="badge">{session.status?.pending}</span>
      {/if}
    </a>
  {/each}
</nav>

<style>
  nav {
    position: fixed;
    bottom: 0;
    left: 0;
    right: 0;
    z-index: 20;
    display: flex;
    min-height: var(--nav-row);
    background: var(--surface);
    border-top: 1px solid var(--border);
    padding-bottom: env(safe-area-inset-bottom, 0);
  }

  a {
    position: relative;
    flex: 1;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 2px;
    min-height: var(--nav-row);
    color: var(--text-muted);
    text-decoration: none;
    font-size: var(--type-small);
  }

  a.active {
    color: var(--accent);
    background: var(--accent-soft);
  }

  .icon {
    font-size: 20px;
    line-height: 1;
  }

  .badge {
    position: absolute;
    top: 6px;
    right: calc(50% - 28px);
    min-width: 18px;
    height: 18px;
    padding: 0 4px;
    border-radius: var(--radius-pill);
    background: var(--accent);
    color: var(--accent-text);
    font-size: 11px;
    line-height: 18px;
    text-align: center;
  }
</style>
