<script lang="ts">
	import Fab from '$lib/components/ui/Fab.svelte';
	import FingerprintIcon from '$lib/components/icons/FingerprintIcon.svelte';

	interface Props {
		onClick?: () => void;
	}

	let { onClick }: Props = $props();
</script>

<Fab position={{ bottom: '36px', right: '36px' }} {onClick}>
	<FingerprintIcon />
</Fab>
