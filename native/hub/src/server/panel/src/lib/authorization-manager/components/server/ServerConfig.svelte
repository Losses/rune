<script lang="ts">
	import type { IServerConfig } from '$lib/authorization-manager';
	import FluentInput from '$lib/components/ui/FluentInput.svelte';
	import FluentSwitch from '$lib/components/ui/FluentSwitch.svelte';

	interface Props {
		config: IServerConfig;
		onUpdate: (config: IServerConfig) => void;
	}

	let { config, onUpdate }: Props = $props();

	const handleAliasChange = (x: string) => {
		onUpdate({ ...config, alias: x });
	};

	const handleBroadcastChange = (checked: boolean) => {
		onUpdate({ ...config, broadcastEnabled: checked });
	};
</script>

<div>
	<div>
		<FluentInput appearance="filled-lighter" value={config.alias} onChange={handleAliasChange}
			>Server Alias</FluentInput
		>
	</div>

	<div class="broadcast">
		<FluentSwitch
			id="broadcast"
			checked={config.broadcastEnabled}
			onChange={handleBroadcastChange}
			label="Enable Local Network Broadcasting"
		/>
	</div>
</div>

<style>
	.broadcast {
		padding-top: 16px;
	}
</style>
