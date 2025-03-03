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

<div class="space-y-8">
	<!-- Server Configuration Section -->
	<section class="pt-6" transition:slide>
		<ax-elevation style="--elevation-depth: 2">
			<div>
				<fluent-text block size="600"><span>Server Configuration</span></fluent-text>
				<fluent-text block size="400"
					><span>Manage your server settings and broadcasting options</span></fluent-text
				>
			</div>
			<div class="space-y-6">
				<ServerConfig config={serverConfig} onUpdate={resProps.onUpdateConfig} />
			</div>
		</ax-elevation>
	</section>

	<!-- Device Management Section -->
	<section class="pt-6" transition:slide>
		<ax-elevation style="--elevation-depth: 2">
			<div>
				<fluent-text block size="600"><span>Device Management</span></fluent-text>
				<fluent-text block size="400"
					><span>View and manage all connected devices</span></fluent-text
				>
			</div>
			<div class="space-y-6">
				<DeviceList {devices} onUpdateStatus={resProps.onUpdateDeviceStatus} />
			</div>
		</ax-elevation>
	</section>
</div>
