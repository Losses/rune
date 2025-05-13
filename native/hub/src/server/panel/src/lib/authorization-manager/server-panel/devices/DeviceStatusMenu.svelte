<script lang="ts">
	import PermissionIcon from '$lib/components/icons/PermissionIcon.svelte';

	interface Props {
		deviceId: string;
		variant: 'large' | 'small';
		onUpdateStatus: (deviceId: string, status: string) => void;
	}

	let { deviceId, onUpdateStatus, variant }: Props = $props();
</script>

{#if variant == 'small'}
	<fluent-menu
		onchange={(event: Event) => {
			if (event.target) {
				onUpdateStatus(deviceId, (event.target as HTMLElement).getAttribute('value') ?? '');
			}
		}}
	>
		<fluent-menu-button icon-only aria-label="Toggle Menu" slot="trigger"
			><span class="permission-icon"><PermissionIcon /></span></fluent-menu-button
		>

		<fluent-menu-list class="action-list">
			<fluent-menu-item value="Approved">Approve</fluent-menu-item>
			<fluent-menu-item value="Pending">Pending</fluent-menu-item>
			<fluent-menu-item value="Blocked">Block</fluent-menu-item>
		</fluent-menu-list>
	</fluent-menu>
{:else}
	<fluent-menu
		class="card-action"
		onchange={(event: Event) => {
			if (event.target) {
				onUpdateStatus(deviceId, (event.target as HTMLElement).getAttribute('value') ?? '');
			}
		}}
	>
		<fluent-button aria-label="Toggle Menu" slot="trigger" class="card-action-trigger">
			Change Permission
		</fluent-button>

		<fluent-menu-list>
			<fluent-menu-item value="Approved">Approve</fluent-menu-item>
			<fluent-menu-item value="Pending">Pending</fluent-menu-item>
			<fluent-menu-item value="Blocked">Block</fluent-menu-item>
		</fluent-menu-list>
	</fluent-menu>
{/if}

<style>
	.action-list {
		min-width: initial;
	}

	.card-action {
		width: 100%;
		margin-top: 8px;
	}

	.card-action-trigger {
		width: 100%;
	}

	.permission-icon {
		display: contents;
		color: rgba(0, 0, 0, 0.75);
	}
</style>
