<script lang="ts">
	import { onMount } from 'svelte';

	interface Props {
		fingerprint: string;
	}

	let { fingerprint }: Props = $props();
	let buttonSize = $state('large');

	if (fingerprint.length !== 40) {
		fingerprint = fingerprint.padEnd(40, '*');
	}

	function updateButtonSize() {
		if (window.matchMedia('(max-width: 300px)').matches) {
			buttonSize = 'small';
		} else if (window.matchMedia('(max-width: 500px)').matches) {
			buttonSize = 'medium';
		} else {
			buttonSize = 'large';
		}
	}

	onMount(() => {
		updateButtonSize();

		const smallMediaQuery = window.matchMedia('(max-width: 300px)');
		const mediumMediaQuery = window.matchMedia('(max-width: 500px)');

		smallMediaQuery.addEventListener('change', updateButtonSize);
		mediumMediaQuery.addEventListener('change', updateButtonSize);

		return () => {
			smallMediaQuery.removeEventListener('change', updateButtonSize);
			mediumMediaQuery.removeEventListener('change', updateButtonSize);
		};
	});
</script>

<div class="fingerprint-grid">
	{#each Array(20) as _, index}
		<fluent-button size={buttonSize}>
			<span class="fingerprint-text">{fingerprint.slice(index * 2, index * 2 + 2)}</span>
		</fluent-button>
	{/each}
</div>

<style>
	.fingerprint-grid {
		width: fit-content;
		margin: 0 auto;
		padding: 12px;
		display: grid;
		grid-template-columns: repeat(4, 1fr);
		grid-template-rows: repeat(5, 1fr);
		gap: 10px;
	}

	.fingerprint-text {
		font-family: 'Noto Sans Runic', sans-serif;
		font-weight: 400;
		font-style: normal;
	}
</style>
