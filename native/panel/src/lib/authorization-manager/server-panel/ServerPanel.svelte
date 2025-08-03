<script lang="ts">
	import MediaQuery from '$lib/components/ui/MediaQuery.svelte';
	import type { IDevice, IServerConfig } from '$lib/authorization-manager';

	import FingerprintFab from './layout/FingerprintFab.svelte';
	import DashboardHeader from './layout/DashboardHeader.svelte';
	import FingerprintCard from './layout/FingerprintCard.svelte';
	import ServerConfigCard from './server/ServerConfigCard.svelte';
	import DeviceManagementSection from './layout/DeviceManagementSection.svelte';
	import FingerprintDialog from './devices/FingerprintDialog.svelte';

	interface Props {
		serverConfig: IServerConfig;
		devices: IDevice[];
		onUpdateConfig: (config: IServerConfig) => void;
		onUpdateDeviceStatus: (deviceId: string, status: string) => void;
		onDeleteDevice: (fingerprint: string) => void;
	}

	let { serverConfig, devices, onUpdateConfig, onUpdateDeviceStatus, onDeleteDevice }: Props =
		$props();

	let dialog: FingerprintDialog | null = $state(null);
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
					<FingerprintCard onClick={dialog?.show} />
				</section>
			</MediaQuery>
		</div>

		<DeviceManagementSection {devices} onUpdateStatus={onUpdateDeviceStatus} {onDeleteDevice} />

		<MediaQuery query="(max-width: 600px)">
			<FingerprintFab onClick={dialog?.show} />
		</MediaQuery>

		<FingerprintDialog bind:this={dialog} fingerprint={serverConfig.fingerprint} />
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
