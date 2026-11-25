<main>
	<slot />
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
</style>
