<script lang="ts">
	import Card from '$lib/components/ui/Card.svelte';
	import type { IServerConfig } from '$lib/authorization-manager';

	import ServerConfig from './ServerConfig.svelte';

	interface Props {
		config: IServerConfig;
		onUpdate: (config: IServerConfig) => void;
	}

	let { config, onUpdate }: Props = $props();
</script>

<Card
	headerText="Server Configuration"
	headerSubtitle="Manage your server settings and broadcasting options"
>
	<ServerConfig {config} {onUpdate} />
</Card>
