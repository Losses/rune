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

<div class="space-y-6">
	<div class="flex flex-col gap-2 sm:grid sm:grid-cols-2 sm:items-center">
		<fluent-text-input id="serverAlias" value={config.alias} onchange={handleAliasChange}
			>Server Alias</fluent-text-input
		>
	</div>

	<div class="flex items-center gap-2">
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
