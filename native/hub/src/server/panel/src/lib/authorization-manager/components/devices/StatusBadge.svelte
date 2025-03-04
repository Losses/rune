<script lang="ts">
	interface Props {
		status: string;
	}

	let { status }: Props = $props();

	const getBadgeColor = (status: string) => {
		switch (status) {
			case 'approved':
				return 'success';
			case 'pending':
				return 'warning';
			case 'blocked':
				return 'danger';
			default:
				return 'brand';
		}
	};
</script>

<fluent-badge class="badge" appearance="filled" color={getBadgeColor(status)}>
	{status}
</fluent-badge>

<style>
	.badge {
		width: 72px;
	}
</style>
