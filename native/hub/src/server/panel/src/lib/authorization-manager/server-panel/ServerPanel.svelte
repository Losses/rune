<script lang="ts">
	import MediaQuery from '$lib/components/ui/MediaQuery.svelte';
	import type { IDevice, IServerConfig } from '$lib/authorization-manager';

	import FingerprintFab from './layout/FingerprintFab.svelte';
	import DashboardHeader from './layout/DashboardHeader.svelte';
	import FingerprintCard from './layout/FingerprintCard.svelte';
	import ServerConfigCard from './server/ServerConfigCard.svelte';
	import DeviceManagementSection from './layout/DeviceManagementSection.svelte';

	interface Props {
		serverConfig: IServerConfig;
		devices: IDevice[];
		onUpdateConfig: (config: IServerConfig) => void;
		onUpdateDeviceStatus: (deviceId: string, status: string) => void;
	}

	let { serverConfig, devices, onUpdateConfig, onUpdateDeviceStatus }: Props = $props();
</script>

<div>
	<DashboardHeader />
	<main>
		<div class="first-row">
			<section class="server-config-card">
				<ServerConfigCard config={serverConfig} onUpdate={onUpdateConfig} />
			</section>

			<MediaQuery query="(min-width: 600px)">
				<section class="fingerprint-card">
					<FingerprintCard />
				</section>
			</MediaQuery>
		</div>

		<DeviceManagementSection {devices} onUpdateStatus={onUpdateDeviceStatus} />

		<MediaQuery query="(max-width: 600px)">
			<FingerprintFab />
		</MediaQuery>
	</main>
</div>

<style>
	main {
		max-width: 768px;
		margin: -120px auto 20px auto;
	}

	@media screen and (max-width: 600px) {
		main {
			max-width: 400px;
		}
	}

	.first-row {
		display: flex;
		align-items: stretch;
	}

	.server-config-card {
		flex: 1;
	}

	.fingerprint-card {
		margin-left: 24px;
		display: flex;
		align-items: stretch;
	}
</style>
