<script lang="ts">
	import MediaQuery from '$lib/components/ui/MediaQuery.svelte';
	import Card from '$lib/components/ui/Card.svelte';
	import type { IDevice } from '$lib/authorization-manager';
	import DeviceTable from '../devices/DeviceTable.svelte';
	import DeviceCard from '../devices/DeviceCard.svelte';

	interface Props {
		devices: IDevice[];
		onUpdateStatus: (deviceId: string, status: string) => void;
	}

	let { devices, onUpdateStatus }: Props = $props();
</script>

<section>
	<MediaQuery query="(min-width: 600px)">
		<Card headerText="Device Management" headerSubtitle="View and manage all connected devices">
			<div>
				<DeviceTable {devices} {onUpdateStatus} />
			</div>
		</Card>
	</MediaQuery>

	<MediaQuery query="(max-width: 600px)">
		{#each devices as device}
			<Card>
				<DeviceCard {device} {onUpdateStatus} />
			</Card>
		{/each}
	</MediaQuery>
</section>
