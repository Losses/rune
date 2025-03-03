<script lang="ts">
	type ServerConfig = {
		alias: string;
		broadcastEnabled: boolean;
	};

	type Props = {
		config: ServerConfig;
		onUpdate: (config: ServerConfig) => void;
	};

	let { config, onUpdate }: Props = $props();

	/** Handle alias change */
	const handleAliasChange = (event: Event) => {
		const input = event.target as HTMLInputElement;
		onUpdate({ ...config, alias: input.value });
	};

	/** Handle broadcast toggle */
	const handleBroadcastChange = (checked: boolean) => {
		onUpdate({ ...config, broadcastEnabled: checked });
	};

	// bind:checked={config.broadcastEnabled}
</script>

<div>
	<div>
		<fluent-text-input id="serverAlias" value={config.alias} onchange={handleAliasChange}
			>Server Alias</fluent-text-input
		>
	</div>

	<div>
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
