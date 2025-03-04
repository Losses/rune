<script lang="ts">
	import DeviceTableRow from './DeviceTableRow.svelte';
	import type { IDevice } from '$lib/authorization-manager';

	interface Props {
		devices: IDevice[];
		onUpdateStatus: (deviceId: string, status: string) => void;
	}

	let { devices, onUpdateStatus }: Props = $props();
</script>

<div>
	<table class="device-list">
		<thead>
			<tr>
				<th><fluent-text weight="bold" class="header-text">Device Name</fluent-text></th>
				<th><fluent-text weight="bold" class="header-text">Fingerprint</fluent-text></th>
				<th><fluent-text weight="bold" class="header-text">Status</fluent-text></th>
				<th><fluent-text weight="bold" class="header-text">Last Seen</fluent-text></th>
				<th><fluent-text weight="bold" class="header-text"></fluent-text></th>
			</tr>
		</thead>
		<tbody>
			{#each devices as device}
				<DeviceTableRow {device} {onUpdateStatus} />
			{/each}
		</tbody>
	</table>
</div>

<style>
	.device-list {
		width: 100%;
		border-collapse: collapse;
		margin-top: 24px;
	}

	.device-list th {
		padding: 16px;
		text-align: start;
		vertical-align: center;
	}

	.header-text {
		opacity: 0.5;
	}

	.device-list tr {
		border-bottom: 1px solid #e4e4e4;
		transition: background-color 100ms;
	}

	.device-list tr:hover {
		background-color: #fafafa;
	}
</style>
