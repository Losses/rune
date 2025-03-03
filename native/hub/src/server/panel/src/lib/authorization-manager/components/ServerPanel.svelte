<script lang="ts">
	import { slide } from 'svelte/transition';

	import ServerConfig from './server/ServerConfig.svelte';
	import DeviceList from './devices/DeviceList.svelte';

	// Renamed type to avoid conflict with component name
	type ServerConfigType = {
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

	type Props = {
		serverConfig: ServerConfigType;
		devices: Device[];
		onUpdateConfig: (config: ServerConfigType) => void;
		onUpdateDeviceStatus: (deviceId: string, status: string) => void;
	};

	let { serverConfig, devices, ...resProps }: Props = $props();
</script>

<div class="main">
	<!-- Server Configuration Section -->
	<section transition:slide>
		<ax-elevation class="card">
			<div class="card-content">
				<header class="card-header">
					<fluent-text block size="600" as="h1" weight="bold">Server Configuration</fluent-text>
					<fluent-text block size="400"
						><span>Manage your server settings and broadcasting options</span></fluent-text
					>
				</header>
				<div>
					<ServerConfig config={serverConfig} onUpdate={resProps.onUpdateConfig} />
				</div>
			</div>
		</ax-elevation>
	</section>

	<!-- Device Management Section -->
	<section transition:slide>
		<ax-elevation class="card">
			<div class="card-content">
				<header class="card-header">
					<fluent-text block size="600" as="h1" weight="bold">Device Management</fluent-text>
					<fluent-text block size="400"
						><span>View and manage all connected devices</span></fluent-text
					>
				</header>
				<div>
					<DeviceList {devices} onUpdateStatus={resProps.onUpdateDeviceStatus} />
				</div>
			</div>
		</ax-elevation>
	</section>
</div>

<style>
	.main {
		max-width: 768px;
		margin: 20px auto;
	}

	.card {
		width: 100%;
		margin: 12px 0;
		background: white;
		--elevation-depth: 2;
	}

	.card-content {
		padding: 12px 24px;
	}

	.card-header {
		margin-top: 8px;
		margin-bottom: 12px;
	}
</style>
