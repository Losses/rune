<script lang="ts">
	import { format } from 'timeago.js';
	import StatusBadge from './StatusBadge.svelte';
	import DeviceStatusMenu from './DeviceStatusMenu.svelte';
	import type { IDevice } from '$lib/authorization-manager';
	import FingerprintIcon from '$lib/components/icons/FingerprintIcon.svelte';
	import FluentButton from '$lib/components/ui/FluentButton.svelte';
	import FingerprintDialog from './FingerprintDialog.svelte';
	import RemoveIcon from '$lib/components/icons/RemoveIcon.svelte';

	interface Props {
		device: IDevice;
		isLast?: boolean;
		onUpdateStatus: (deviceId: string, status: string) => void;
		onDeleteDevice: (fingerprint: string) => void;
	}

	let { device, onUpdateStatus, onDeleteDevice, isLast }: Props = $props();

	let dialog: FingerprintDialog | null = $state(null);
</script>

<tr class={isLast ? 'is-last' : ''}>
	<th>
		<fluent-text weight="bold">{device.name}</fluent-text>
		<div class="fingerprint-button">
			<FluentButton
				iconOnly={true}
				size="small"
				ariaLabel="Display Fingerprints"
				onClick={dialog?.show}
			>
				<span class="fingerprint-icon">
					<FingerprintIcon />
				</span>
			</FluentButton>
		</div>
		<FingerprintDialog bind:this={dialog} fingerprint={device.fingerprint} />
	</th>
	<th>
		<div class="badge">
			<StatusBadge status={device.status} />
		</div>
	</th>
	<th>
		<fluent-text>
			{format(device.lastSeen, 'en_US')}
		</fluent-text>
	</th>
	<th>
		<DeviceStatusMenu deviceId={device.id} {onUpdateStatus} variant="small" />
		<FluentButton
			iconOnly={true}
			ariaLabel="Delete Device"
			onClick={() => onDeleteDevice(device.fingerprint)}
		>
			<span class="delete-icon">
				<RemoveIcon />
			</span>
		</FluentButton>
	</th>
</tr>

<style>
	th {
		padding: 16px;
		text-align: start;
		vertical-align: center;
	}

	tr {
		border-bottom: 1px solid #e4e4e4;
		transition: background-color 100ms;
	}

	tr.is-last {
		border-bottom: 0;
	}

	tr:hover {
		background-color: #fafafa;
	}

	.fingerprint-button {
		margin-left: 8px;
		display: inline-block;
	}

	.fingerprint-icon {
		display: contents;
		color: rgba(0, 0, 0, 0.75);
	}

	.badge {
		transform: translateY(4px);
	}

	.delete-icon {
		display: contents;
		color: rgba(0, 0, 0, 0.75);
	}
</style>
