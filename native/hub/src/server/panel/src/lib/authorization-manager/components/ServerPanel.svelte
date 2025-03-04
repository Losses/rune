<script lang="ts">
	import { slide } from 'svelte/transition';

	import Fab from '$lib/components/ui/Fab.svelte';
	import RuneLogo from '$lib/components/icons/RuneLogo.svelte';
	import MediaQuery from '$lib/components/ui/MediaQuery.svelte';
	import FingerprintIcon from '$lib/components/icons/FingerprintIcon.svelte';
	import type { IDevice, IServerConfig } from '$lib/authorization-manager';

	import DeviceCard from './devices/DeviceCard.svelte';
	import DeviceTable from './devices/DeviceTable.svelte';
	import ServerConfig from './server/ServerConfig.svelte';

	interface Props {
		serverConfig: IServerConfig;
		devices: IDevice[];
		onUpdateConfig: (config: IServerConfig) => void;
		onUpdateDeviceStatus: (deviceId: string, status: string) => void;
	}

	let { serverConfig, devices, ...resProps }: Props = $props();
</script>

<div>
	<header class="title">
		<div class="logo">
			<RuneLogo />
		</div>
	</header>
	<main>
		<div class="first-row">
			<section class="server-config-card">
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
				<section class="fingerprint-card">
					<ax-elevation class="card">
						<fluent-button appearance="primary" shape="square" class="reveal-fingerprint-button"
							><div class="reveal-fingerprint-tag">
								<FingerprintIcon />
								<div class="label">
									<fluent-text size="500" class="reveal" weight="bold">Reveal</fluent-text>
									<fluent-text size="200">Device Fingerprint</fluent-text>
								</div>
							</div>
						</fluent-button>
					</ax-elevation>
				</section>
			</MediaQuery>
		</div>

		<MediaQuery query="(min-width: 600px)">
			<section>
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
				<section>
					<ax-elevation class="card">
						<div class="card-content">
							<DeviceCard {device} onUpdateStatus={resProps.onUpdateDeviceStatus} />
						</div>
					</ax-elevation>
				</section>
			{/each}
		</MediaQuery>

		<MediaQuery query="(max-width: 600px)">
			<Fab position={{ bottom: '36px', right: '36px' }} onClick={() => {}}>
				<FingerprintIcon />
			</Fab>
		</MediaQuery>
	</main>
</div>

<style>
	.title {
		width: 100%;
		min-height: 200px;
		height: 45vh;
		color: white;
		background: url('/background.png');
		background-size: cover;
		background-position: bottom left;

		display: flex;
		justify-content: center;
		align-items: center;
	}

	.logo {
		width: 200px;
		transform: translateY(-40px);
	}

	main {
		max-width: 768px;
		margin: -120px auto 20px auto;
	}

	@media screen and (max-width: 600px) {
		main {
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

	.fingerprint-card .card {
		min-width: 180px;
		display: block;
	}

	.reveal-fingerprint-button {
		width: 100%;
		height: 100%;
	}

	.reveal-fingerprint-tag {
		display: flex;
		flex-direction: column;
		justify-content: center;
		align-items: center;
		font-size: 48px;
	}

	.reveal-fingerprint-tag .label {
		margin-top: 12px;
	}

	.reveal-fingerprint-tag .label fluent-text {
		display: block;
		text-align: center;
	}
</style>
