<script lang="ts">
	import { onMount, type Snippet } from 'svelte';

	interface Props {
		query: string;
		children: Snippet;
	}

	let { query, children }: Props = $props();

	let matches = $state(window.matchMedia(query).matches);
	let wasMounted = false;

	let mql: MediaQueryList;
	let mqlListener: (e: MediaQueryListEvent) => void;

	onMount(() => {
		wasMounted = true;
		addNewListener(query);

		return () => {
			removeActiveListener();
		};
	});

	$effect(() => {
		if (!wasMounted) return;
		removeActiveListener();
		addNewListener(query);
	});

	function addNewListener(query: string): void {
		mql = window.matchMedia(query);
		mqlListener = (v: MediaQueryListEvent) => (matches = v.matches);

		mql.addEventListener('change', mqlListener);
	}

	function removeActiveListener(): void {
		if (mql && mqlListener) {
			mql.removeEventListener('change', mqlListener);
		}
	}
</script>

{#if matches}
	{@render children()}
{/if}
