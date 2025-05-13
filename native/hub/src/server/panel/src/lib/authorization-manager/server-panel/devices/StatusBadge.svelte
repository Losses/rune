<script lang="ts">
	import MediaQuery from '$lib/components/ui/MediaQuery.svelte';

	interface Props {
		status: string;
	}

	let { status }: Props = $props();

	const getBadgeColor = (status: string) => {
		switch (status) {
			case 'Approved':
				return 'success';
			case 'Pending':
				return 'warning';
			case 'Blocked':
				return 'danger';
			default:
				return 'brand';
		}
	};
</script>

<MediaQuery query="(min-width: 320px)">
	<fluent-badge class="badge" appearance="filled" color={getBadgeColor(status)}>
		{status}
	</fluent-badge>
</MediaQuery>

<MediaQuery query="(max-width: 320px)">
	<fluent-badge appearance="filled" color={getBadgeColor(status)} size="extra-small"> </fluent-badge>
</MediaQuery>

<style>
	.badge {
		width: 72px;
	}
</style>
