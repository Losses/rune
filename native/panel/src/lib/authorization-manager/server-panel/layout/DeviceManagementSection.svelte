<script lang="ts">
	import MediaQuery from '$lib/components/ui/MediaQuery.svelte';
	import Card from '$lib/components/ui/Card.svelte';
	import type { IDevice } from '$lib/authorization-manager';

	import DeviceTable from '../devices/DeviceTable.svelte';
	import DeviceCard from '../devices/DeviceCard.svelte';
	import NoDevices from '../devices/NoDevices.svelte';

	interface Props {
		devices: IDevice[];
		onUpdateStatus: (deviceId: string, status: string) => void;
		onDeleteDevice: (fingerprint: string) => void;
	}

	let { devices, onUpdateStatus, onDeleteDevice }: Props = $props();
</script>

<section>
	<MediaQuery query="(min-width: 600px)">
		<Card headerText="Device Management" headerSubtitle="View and manage all connected devices">
			<div>
				<DeviceTable {devices} {onUpdateStatus} {onDeleteDevice} />
			</div>
		</Card>
	</MediaQuery>

	<MediaQuery query="(max-width: 600px)">
		{#if devices.length == 0}
			<Card>
				<NoDevices />
			</Card>
		{/if}
		{#each devices as device}
			<Card>
				<DeviceCard {device} {onUpdateStatus} {onDeleteDevice} />
			</Card>
		{/each}
	</MediaQuery>
</section>
