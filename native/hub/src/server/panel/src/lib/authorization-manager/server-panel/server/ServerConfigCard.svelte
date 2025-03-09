<script lang="ts">
	import { get } from 'svelte/store';
	import { debounce, throttle } from 'lodash-es';

	import Card from '$lib/components/ui/Card.svelte';
	import { token } from '$lib/stores/token.svelte';
	import { type IServerConfig } from '$lib/authorization-manager';

	import ServerConfig from './ServerConfig.svelte';

	interface Props {
		config: IServerConfig;
		onUpdate: (config: IServerConfig) => void;
	}

	let { config, onUpdate }: Props = $props();

	let isUpdatingAlias = $state(false);
	let isBroadcastLoading = $state(false);

	const doUpdate = debounce(async (alias: string) => {
		try {
			onUpdate({
				...config,
				alias
			});
			const res = await fetch('/panel/alias', {
				method: 'PUT',
				body: JSON.stringify({ alias }),
				headers: {
					Authorization: `Bearer ${get(token)}`,
					'Content-Type': 'application/json'
				}
			});

			if (!res.ok) {
				throw new Error('Request failed');
			}

			isUpdatingAlias = false;
		} catch (e) {
			doUpdate(alias);
		}
	}, 2000);

	const handleAliasUpdate = (alias: string) => {
		isUpdatingAlias = true;
		doUpdate(alias);
	};

	const handleBroadcastUpdate = throttle(
		async (broadcastEnabled: boolean) => {
			isBroadcastLoading = true;
			const originalState = config.broadcastEnabled;
			onUpdate({
				...config,
				broadcastEnabled
			});

			try {
				await fetch('/panel/broadcast', {
					method: 'PUT',
					body: JSON.stringify({ enabled: broadcastEnabled }),
					headers: {
						Authorization: `Bearer ${get(token)}`,
						'Content-Type': 'application/json'
					}
				});
			} catch (e) {
				onUpdate({
					...config,
					broadcastEnabled: originalState
				});
			} finally {
				setTimeout(() => {
					isBroadcastLoading = false;
				}, 2000);
			}
		},
		2000,
		{ trailing: false }
	);
</script>

<Card
	headerText="Server Configuration"
	headerSubtitle="Manage your server settings and broadcasting options"
>
	<ServerConfig
		{config}
		onAliasUpdate={handleAliasUpdate}
		onBroadcastUpdate={handleBroadcastUpdate}
		{isUpdatingAlias}
		{isBroadcastLoading}
	/>
</Card>
