<script lang="ts">
	import { format } from 'timeago.js';

	import FluentButton from '$lib/components/ui/FluentButton.svelte';
	import { type IDevice } from '$lib/authorization-manager';

	import StatusBadge from './StatusBadge.svelte';
	import DeviceStatusMenu from './DeviceStatusMenu.svelte';
	import FingerprintDialog from './FingerprintDialog.svelte';

	interface Props {
		device: IDevice;
		onUpdateStatus: (deviceId: string, status: string) => void;
		onDeleteDevice: (fingerprint: string) => void;
	}

	let { device, onUpdateStatus, onDeleteDevice }: Props = $props();

	let dialog: FingerprintDialog | null = $state(null);
</script>

<div class="card">
	<div class="card-title">
		<fluent-text as="h3" size="500" weight="bold" class="device-name">{device.name}</fluent-text>
		<div class="badge"><StatusBadge status={device.status} /></div>
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

	<FluentButton fullWidth onClick={dialog?.show}>Inspect Fingerprint</FluentButton>
	<DeviceStatusMenu deviceId={device.id} {onUpdateStatus} variant="large" />
	<FluentButton fullWidth onClick={() => onDeleteDevice(device.fingerprint)}
		>Delete Device</FluentButton
	>

	<FingerprintDialog bind:this={dialog} fingerprint={device.fingerprint} />
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

	.card-title .badge {
		flex-shrink: 0;
	}

	.field {
		margin: 12px 0;
	}

	.field-subtitle {
		opacity: 0.5;
	}
</style>
