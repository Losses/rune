<script lang="ts">
	import FluentInput from '$lib/components/ui/FluentInput.svelte';
	import FluentSwitch from '$lib/components/ui/FluentSwitch.svelte';
	import { type IServerConfig } from '$lib/authorization-manager';
	import SyncIcon from '$lib/components/icons/SyncIcon.svelte';

	interface Props {
		config: IServerConfig;
		onAliasUpdate: (alias: string) => void;
		onBroadcastUpdate: (enabled: boolean) => void;
		isUpdatingAlias: boolean;
		isBroadcastLoading: boolean;
	}

	let { config, onAliasUpdate, onBroadcastUpdate, isBroadcastLoading, isUpdatingAlias }: Props =
		$props();

	const handleAliasChange = (x: string) => {
		onAliasUpdate(x);
	};

	const handleBroadcastChange = (checked: boolean) => {
		onBroadcastUpdate(checked);
	};
</script>

{#snippet syncIcon()}
	{#if isUpdatingAlias}
		<SyncIcon />
	{/if}
{/snippet}

<div>
	<div>
		<FluentInput
			appearance="filled-lighter"
			value={config.alias}
			onChange={handleAliasChange}
			end={syncIcon}
		>
			Server Alias
		</FluentInput>
	</div>

	<div class="broadcast">
		<FluentSwitch
			id="broadcast"
			checked={config.broadcastEnabled}
			onChange={handleBroadcastChange}
			disabled={isBroadcastLoading}
			label="Enable Local Network Broadcasting"
		/>
	</div>
</div>

<style>
	.broadcast {
		padding-top: 20px;
	}
</style>
