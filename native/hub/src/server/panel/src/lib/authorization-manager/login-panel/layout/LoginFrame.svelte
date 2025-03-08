<script lang="ts">
	import type { Snippet } from 'svelte';

	import RuneLogo from '$lib/components/icons/RuneLogo.svelte';
	import Background from '$lib/components/ui/Background.svelte';

	interface Props {
		children: Snippet;
	}

	let { children }: Props = $props();
</script>

<Background>
	<section class="body">
		<div class="logo">
			<RuneLogo />
		</div>
		{@render children()}
	</section>
</Background>

<style>
	.body {
		display: flex;
		width: 340px;
		justify-content: center;
		align-items: center;
		flex-direction: column;
	}

	.logo {
		width: 160px;
		color: white;
		text-align: center;
		margin-bottom: 24px;
	}
</style>
