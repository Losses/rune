<script lang="ts">
	import LoginCard from './layout/LoginCard.svelte';
	import LoginFrame from './layout/LoginFrame.svelte';

	interface Props {
		onSubmit: (password: string) => void;
	}

	let { onSubmit }: Props = $props();
</script>

<LoginFrame>
	<LoginCard {onSubmit} />
</LoginFrame>
