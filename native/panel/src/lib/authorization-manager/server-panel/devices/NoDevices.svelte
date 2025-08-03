<script>

	import PremiumIconColor from "$lib/components/icons/PremiumIconColor.svelte";

</script>
<div class="no-devices">
	<div class="no-devices-icon"><PremiumIconColor /></div>
	<fluent-text size="700" weight="bold">No Devices</fluent-text>
	<fluent-text>Devices from the LAN will be listed here</fluent-text>
</div>

<style>
	.no-devices {
		padding: 64px 24px;
		display: flex;
		flex-direction: column;
		justify-content: center;
		align-items: center;
	}

	.no-devices-icon {
		font-size: 64px;
	}
</style>
