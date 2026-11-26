<script lang="ts">
  import '../app.css';
  import { onMount } from 'svelte';
  import type { Snippet } from 'svelte';
  import BottomNav from '$lib/components/BottomNav.svelte';
  import StatusStrip from '$lib/components/StatusStrip.svelte';
  import { session } from '$lib/stores/session.svelte';

  let { children }: { children: Snippet } = $props();

  onMount(() => {
    void session.load();
  });
</script>

<StatusStrip />

<main>
  {@render children()}
</main>

<BottomNav />

<style>
  main {
    min-height: calc(100vh - var(--status-height));
  }

  @media (min-width: 1024px) {
    main {
      max-width: 1180px;
      margin: 0 auto;
    }
  }
</style>
