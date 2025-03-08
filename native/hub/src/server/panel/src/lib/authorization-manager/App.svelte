<script lang="ts">
	import { localStore } from '$lib/utils.svelte';

	import LoginPanel from './login-panel/LoginPanel.svelte';
	import ServerPanel from './server-panel/ServerPanel.svelte';

	import type { IDevice, IServerConfig } from '.';

	let token = localStore<string>('token', '');

	let serverConfig: IServerConfig = $state({
		alias: 'Main Server',
		broadcastEnabled: true
	});

	let devices: IDevice[] = $state([
		{
			id: '1',
			name: 'Development Laptop',
			fingerprint: 'ᚿᛕᛄᛷᚠᛋᚹᚶᚿᛕᚻᛥᚺᚷᚲᛋᚶᚿᛕᛄᛷᚠᛋᚹᚶᚿᛕᛄᛷᚠᛅᚻᛥᚺᚷᚲᛋᛋᚹᚶ',
			status: 'approved',
			lastSeen: new Date()
		},
		{
			id: '2',
			name: 'Testing Device',
			fingerprint: 'ᛅᛅᚻᛥᛕᛅᛅᛋᛅᛅᚻᛥᚺᚷᚲᛋᛅᛅᚻᛥᚺᚷᚲᛋᛅᛅᚻᛥᚺᚷᚲᛋᛅᛅᚻᛥᚺᚷᚲᛋ',
			status: 'pending',
			lastSeen: new Date()
		},
		{
			id: '3',
			name: 'Unknown Device',
			fingerprint: 'ᛅᛅᚻᛥᛕᛅᛅᛋᛅᛅᚻᛥᚺᚷᚲᛋᛅᛅᚻᛥᚺᚷᚲᛋᛅᛅᚻᛥᚺᚷᚲᛋᛅᛅᚻᛥᚺᚷᚲᛋ',
			status: 'blocked',
			lastSeen: new Date()
		}
	]);

	/** Handle server config updates */
	const onServerConfigUpdate = (config: IServerConfig) => {
		serverConfig = config;
	};

	/** Handle device status updates */
	const onDeviceStatusUpdate = (deviceId: string, newStatus: string) => {
		devices = devices.map((device) =>
			device.id === deviceId ? { ...device, status: newStatus } : device
		);
	};
</script>

<main>
	{#if token.value == ''}
		<LoginPanel onSubmit={(x) => console.log(x)} />
	{:else}
		<ServerPanel
			{serverConfig}
			{devices}
			onUpdateConfig={onServerConfigUpdate}
			onUpdateDeviceStatus={onDeviceStatusUpdate}
		/>
	{/if}
</main>
