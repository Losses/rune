<script lang="ts">
	import { fade } from 'svelte/transition';
	import ServerPanel from './components/ServerPanel.svelte';

	// Types for our server and device data
	type ServerConfig = {
		alias: string;
		broadcastEnabled: boolean;
	};

	type Device = {
		id: string;
		name: string;
		fingerprint: string;
		status: string;
		lastSeen: Date;
	};

	// Sample data
	let serverConfig: ServerConfig = $state({
		alias: 'Main Server',
		broadcastEnabled: true
	});

	let devices: Device[] = $state([
		{
			id: '1',
			name: 'Development Laptop',
			fingerprint: 'f1:2d:3b:4a:5c:6e',
			status: 'approved',
			lastSeen: new Date()
		},
		{
			id: '2',
			name: 'Testing Device',
			fingerprint: 'a1:b2:c3:d4:e5:f6',
			status: 'pending',
			lastSeen: new Date()
		},
		{
			id: '3',
			name: 'Unknown Device',
			fingerprint: 'x1:y2:z3:w4:v5:u6',
			status: 'blocked',
			lastSeen: new Date()
		}
	]);

	/** Handle server config updates */
	const onServerConfigUpdate = (config: ServerConfig) => {
		serverConfig = config;
	};

	/** Handle device status updates */
	const onDeviceStatusUpdate = (
		deviceId: string,
		newStatus: string
	) => {
		devices = devices.map((device) =>
			device.id === deviceId ? { ...device, status: newStatus } : device
		);
	};
</script>

<div class="bg-background min-h-dvh">
	<main class="container mx-auto p-4 md:p-6" transition:fade>
		<ServerPanel
			{serverConfig}
			{devices}
			onUpdateConfig={onServerConfigUpdate}
			onUpdateDeviceStatus={onDeviceStatusUpdate}
		/>
	</main>
</div>
