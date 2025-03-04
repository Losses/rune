<script lang="ts">
	interface Props {
		fingerprint: string;
	}

	let { fingerprint }: Props = $props();

	if (fingerprint.length !== 40) {
		fingerprint = fingerprint.padEnd(40, '*');
	}

	$inspect(fingerprint);

	const rows = 5;
	const cols = 4;
</script>

<div class="fingerprint-grid">
	{#each Array(20) as _, index}
		<fluent-button size="large">
			<span class="fingerprint-text">{fingerprint.slice(index * 2, index * 2 + 2)}</span>
		</fluent-button>
	{/each}
</div>

<style>
	.fingerprint-grid {
		width: fit-content;
		padding: 12px;
		display: grid;
		grid-template-columns: repeat(4, 1fr);
		grid-template-rows: repeat(5, 1fr);
		gap: 10px;
        margin: 0 auto;
	}

	.fingerprint-text {
		font-family: 'Noto Sans Runic', sans-serif;
		font-weight: 400;
		font-style: normal;
	}
</style>
