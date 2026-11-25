<script lang="ts">
	import RuneLogo from '$lib/components/icons/RuneLogo.svelte';
</script>

<header class="title">
	<div class="logo">
		<RuneLogo />
	</div>
</header>

<style>
	.title {
		width: 100%;
		min-height: 200px;
		height: 45vh;
		color: white;
		background: url('/background.png');
		background-size: cover;
		background-position: bottom left;

		display: flex;
		justify-content: center;
		align-items: center;
	}

	.logo {
		width: 200px;
		transform: translateY(-40px);
	}
</style>
