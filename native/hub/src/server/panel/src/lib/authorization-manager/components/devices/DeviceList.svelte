<script lang="ts">
	import MediaQuery from '$lib/components/ui/MediaQuery.svelte';
	import type { IDevice } from '$lib/authorization-manager';

	import DeviceTable from './DeviceTable.svelte';
	import DeviceCard from './DeviceCard.svelte';

	interface Props {
		devices: IDevice[];
		onUpdateStatus: (deviceId: string, status: string) => void;
	}

	let { devices, onUpdateStatus }: Props = $props();
</script>

<MediaQuery query="(min-width: 600px)">
	<DeviceTable {devices} {onUpdateStatus} />
</MediaQuery>

<MediaQuery query="(max-width: 600px)">
	<div class="device-list-mobile">
		{#each devices as device}
			<DeviceCard {device} {onUpdateStatus} />
		{/each}
	</div>
</MediaQuery>

<style>
	.device-list-mobile {
		margin-top: 24px;
	}
</style>
