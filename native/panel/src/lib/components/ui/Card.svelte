<script lang="ts">
	import type { Snippet } from 'svelte';

	interface Props {
		headerText?: string;
		headerSubtitle?: string;
		noPadding?: boolean;
		children: Snippet;
	}

	let { headerText, headerSubtitle, noPadding, children }: Props = $props();
</script>

<ax-elevation class="card">
	<div class={noPadding ? 'no-padding' : 'card-content'}>
		{#if headerText}
			<header class="card-header">
				<fluent-text block size="600" as="h1" weight="bold">{headerText}</fluent-text>
				{#if headerSubtitle}
					<fluent-text block size="300" class="card-header-subtitle">{headerSubtitle}</fluent-text>
				{/if}
			</header>
		{/if}
		<div>
			{@render children()}
		</div>
	</div>
</ax-elevation>

<style>
	.card {
		width: 100%;
		margin: 12px 0;
		background: white;
		--elevation-depth: 2;
	}

	.card-content {
		padding: 20px 24px 18px 24px;
	}

	.no-padding {
		display: flex;
		height: 100%;
	}

	.card-header {
		margin-top: 8px;
		margin-bottom: 16px;
	}

	.card-header-subtitle {
		opacity: 0.5;
	}
</style>
