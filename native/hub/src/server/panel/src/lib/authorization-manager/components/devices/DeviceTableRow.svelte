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

<tr>
	<th><fluent-text weight="bold">{device.name}</fluent-text></th>
	<th><fluent-text font="monospace" truncate="true">{device.fingerprint}</fluent-text></th>
	<th>
		<StatusBadge status={device.status} />
	</th>
	<th><fluent-text>{format(device.lastSeen, 'en_US')}</fluent-text></th>
	<th>
		<DeviceStatusMenu deviceId={device.id} {onUpdateStatus} variant="small" />
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

	tr:hover {
		background-color: #fafafa;
	}
</style>
