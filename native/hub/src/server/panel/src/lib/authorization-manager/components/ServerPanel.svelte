<script lang="ts">
	import { slide } from 'svelte/transition';

	import MediaQuery from '$lib/components/ui/MediaQuery.svelte';
	import type { IDevice, IServerConfig } from '$lib/authorization-manager';

	import ServerConfig from './server/ServerConfig.svelte';
	import DeviceTable from './devices/DeviceTable.svelte';
	import DeviceCard from './devices/DeviceCard.svelte';

	interface Props {
		serverConfig: IServerConfig;
		devices: IDevice[];
		onUpdateConfig: (config: IServerConfig) => void;
		onUpdateDeviceStatus: (deviceId: string, status: string) => void;
	}

	let { serverConfig, devices, ...resProps }: Props = $props();
</script>

<div class="main">
	<!-- Server Configuration Section -->
	<section transition:slide>
		<ax-elevation class="card">
			<div class="card-content">
				<header class="card-header">
					<fluent-text block size="600" as="h1" weight="bold">Server Configuration</fluent-text>
					<fluent-text block size="300" class="card-header-subtitle"
						>Manage your server settings and broadcasting options</fluent-text
					>
				</header>
				<div>
					<ServerConfig config={serverConfig} onUpdate={resProps.onUpdateConfig} />
				</div>
			</div>
		</ax-elevation>
	</section>

	<MediaQuery query="(min-width: 600px)">
		<section transition:slide>
			<ax-elevation class="card">
				<div class="card-content">
					<header class="card-header">
						<fluent-text block size="600" as="h1" weight="bold">Device Management</fluent-text>
						<fluent-text block size="300" class="card-header-subtitle"
							>View and manage all connected devices</fluent-text
						>
					</header>
					<div>
						<DeviceTable {devices} onUpdateStatus={resProps.onUpdateDeviceStatus} />
					</div>
				</div>
			</ax-elevation>
		</section>
	</MediaQuery>

	<MediaQuery query="(max-width: 600px)">
		{#each devices as device}
			<section transition:slide>
				<ax-elevation class="card">
					<div class="card-content">
						<DeviceCard {device} onUpdateStatus={resProps.onUpdateDeviceStatus} />
					</div>
				</ax-elevation>
			</section>
		{/each}
	</MediaQuery>
</div>

<style>
	.main {
		max-width: 768px;
		margin: 20px auto;
	}

	@media screen and (max-width: 600px) {
		.main {
			max-width: 400px;
		}
	}

	.card {
		width: 100%;
		margin: 12px 0;
		background: white;
		--elevation-depth: 2;
	}

	.card-content {
		padding: 20px 24px 18px 24px;
	}

	.card-header {
		margin-top: 8px;
		margin-bottom: 16px;
	}

	.card-header-subtitle {
		opacity: 0.5;
	}
</style>
