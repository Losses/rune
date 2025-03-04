<script lang="ts">
	import { format } from 'timeago.js';
	import StatusBadge from './StatusBadge.svelte';
	import DeviceStatusMenu from './DeviceStatusMenu.svelte';
	import type { IDevice } from '$lib/authorization-manager';
	import FingerprintFigure from './FingerprintFigure.svelte';

	interface Props {
		device: IDevice;
		onUpdateStatus: (deviceId: string, status: string) => void;
	}

	let { device, onUpdateStatus }: Props = $props();

	let dialog: HTMLDialogElement;
</script>

<div class="card">
	<div class="card-title">
		<fluent-text as="h3" size="500" weight="bold" class="device-name">{device.name}</fluent-text>
		<StatusBadge status={device.status} />
	</div>
	<div>
		<div class="field">
			<div>
				<fluent-text class="field-subtitle">Last Seen:</fluent-text>
			</div>
			<div>
				<fluent-text>{format(device.lastSeen, 'en_US')}</fluent-text>
			</div>
		</div>
	</div>

	<fluent-dialog id={`dialog-${device.fingerprint}`} bind:this={dialog}>
		<fluent-dialog-body>
			<FingerprintFigure fingerprint={device.fingerprint} />

			<div slot="title">Fingerprint</div>
		</fluent-dialog-body>
	</fluent-dialog>

	<!-- svelte-ignore a11y_click_events_have_key_events -->
	<!-- svelte-ignore a11y_no_static_element_interactions -->
	<fluent-button class="fingerprint-button" onclick={() => dialog.show()}>
		Inspect Fingerprint
	</fluent-button>

	<DeviceStatusMenu deviceId={device.id} {onUpdateStatus} variant="large" />
</div>

<style>
	.card-title {
		display: flex;
		align-items: center;
		justify-content: space-between;
	}

	.card-title .device-name {
		margin-right: 12px;
	}

	.field {
		margin: 12px 0;
	}

	.field-subtitle {
		opacity: 0.5;
	}

	.fingerprint-button {
		width: 100%;
	}
</style>
