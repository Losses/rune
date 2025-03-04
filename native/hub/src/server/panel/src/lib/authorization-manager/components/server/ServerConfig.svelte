<script lang="ts">
	import type { IServerConfig } from '$lib/authorization-manager';

	interface Props {
		config: IServerConfig;
		onUpdate: (config: IServerConfig) => void;
	}

	let { config, onUpdate }: Props = $props();

	const handleAliasChange = (event: Event) => {
		const input = event.target as HTMLInputElement;
		onUpdate({ ...config, alias: input.value });
	};

	const handleBroadcastChange = (checked: boolean) => {
		onUpdate({ ...config, broadcastEnabled: checked });
	};

	// bind:checked={config.broadcastEnabled}
</script>

<div>
	<div>
		<fluent-text-input
			id="serverAlias"
			appearance="filled-lighter"
			value={config.alias}
			onchange={handleAliasChange}>Server Alias</fluent-text-input
		>
	</div>

	<div class="broadcast">
		<fluent-field label-position="after">
			<!-- svelte-ignore a11y_label_has_associated_control -->
			<label slot="label">Enable Local Network Broadcasting</label>
			<fluent-switch
				id="broadcast"
				onchange={(e: InputEvent) =>
					handleBroadcastChange((e.currentTarget as HTMLInputElement).checked)}
				slot="input"
			></fluent-switch>
		</fluent-field>
	</div>
</div>

<style>
	.broadcast {
		padding-top: 16px;
	}
</style>
