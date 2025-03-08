<script lang="ts">
	import RuneLogo from '$lib/components/icons/RuneLogo.svelte';
	import type { Snippet } from 'svelte';

	interface Props {
		children: Snippet;
	}

	let { children }: Props = $props();
</script>

<main>
	<section class="body">
		<div class="logo">
			<RuneLogo />
		</div>
		{@render children()}
	</section>
</main>

<style>
	main {
		width: 100vw;
		height: 100vh;
		background: url('/background.png');
		background-size: cover;
		background-position: bottom left;

		display: flex;
		justify-content: center;
		align-items: center;
	}

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
