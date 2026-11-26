<script lang="ts">
  import { session } from '$lib/stores/session.svelte';
</script>

<div class="strip" class:offline={session.status?.offline ?? true}>
  <span class="dot"></span>
  <span class="label">
    {session.status?.offline
      ? 'OFFLINE · recorded, not verified'
      : 'Connected · sales sync automatically'}
  </span>
  <span class="spacer"></span>
  <span class="chip">{session.status?.pending ?? 0} queued</span>
</div>

<style>
  .strip {
    position: sticky;
    top: 0;
    z-index: 20;
    display: flex;
    align-items: center;
    gap: var(--space-sm);
    min-height: var(--status-height);
    padding: var(--space-xs) var(--space-lg);
    background: var(--surface-alt);
    border-bottom: 1px solid var(--border);
    color: var(--text-muted);
    font-size: var(--type-status);
  }

  .strip.offline {
    background: var(--accent-soft);
    border-bottom-color: var(--accent);
    color: var(--text);
  }

  .dot {
    width: 8px;
    height: 8px;
    border-radius: var(--radius-pill);
    background: var(--success);
    flex: none;
  }

  .offline .dot {
    background: var(--accent);
  }

  .label {
    font-weight: 600;
    letter-spacing: 0.01em;
  }

  .spacer {
    flex: 1;
  }

  .chip {
    padding: 2px var(--space-sm);
    border-radius: var(--radius-pill);
    background: var(--surface);
    font-size: var(--type-small);
    white-space: nowrap;
  }
</style>
