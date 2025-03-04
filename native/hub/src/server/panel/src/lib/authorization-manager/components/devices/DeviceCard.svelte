<script lang="ts">
	import { format } from 'timeago.js';
	import StatusBadge from './StatusBadge.svelte';
	import DeviceStatusMenu from './DeviceStatusMenu.svelte';
	import type { IDevice } from '$lib/authorization-manager';

	interface Props {
		device: IDevice;
		onUpdateStatus: (deviceId: string, status: string) => void;
	}

	let { device, onUpdateStatus }: Props = $props();
</script>

<div class="card">
	<div class="card-title">
		<fluent-text as="h3" size="500" weight="bold" class="device-name">{device.name}</fluent-text>
		<StatusBadge status={device.status} />
	</div>
	<div>
		<div class="field">
			<div>
				<fluent-text class="field-subtitle">Fingerprint:</fluent-text>
			</div>
			<div>
				<fluent-text font="monospace">{device.fingerprint}</fluent-text>
			</div>
		</div>
		<div class="field">
			<div>
				<fluent-text class="field-subtitle">Last Seen:</fluent-text>
			</div>
			<div>
				<fluent-text>{format(device.lastSeen, 'en_US')}</fluent-text>
			</div>
		</div>
	</div>

	<DeviceStatusMenu deviceId={device.id} {onUpdateStatus} variant="large" />
</div>

<style>
	.card {
		border: 1px solid #eeeeee;
		margin: 16px 0;
		padding: 16px 24px;
	}

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
</style>
