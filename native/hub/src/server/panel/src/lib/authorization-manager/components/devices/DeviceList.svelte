<script lang="ts">
	import { format } from 'timeago.js';

	import MediaQuery from '$lib/components/ui/MediaQuery.svelte';
	import type { IDevice } from '$lib/authorization-manager';

	interface Props {
		devices: IDevice[];
		onUpdateStatus: (deviceId: string, status: string) => void;
	}

	let { devices, onUpdateStatus }: Props = $props();

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

<MediaQuery query="(min-width: 600px)">
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
					<tr>
						<th><fluent-text weight="bold">{device.name}</fluent-text></th>
						<th><fluent-text font="monospace" truncate="true">{device.fingerprint}</fluent-text></th
						>
						<th>
							<fluent-badge class="badge" appearance="filled" color={getBadgeColor(device.status)}>
								{device.status}
							</fluent-badge>
						</th>
						<th><fluent-text>{format(device.lastSeen, 'en_US')}</fluent-text></th>
						<th>
							<fluent-menu
								onchange={(event: Event) => {
									if (event.target) {
										onUpdateStatus(
											device.id,
											(event.target as HTMLElement).getAttribute('value') ?? ''
										);
									}
								}}
							>
								<fluent-menu-button icon-only aria-label="Toggle Menu" slot="trigger">
								</fluent-menu-button>

								<fluent-menu-list>
									<fluent-menu-item value="approved">Approve</fluent-menu-item>
									<fluent-menu-item value="pending">Pending</fluent-menu-item>
									<fluent-menu-item value="blocked">Block</fluent-menu-item>
								</fluent-menu-list>
							</fluent-menu>
						</th>
					</tr>
				{/each}
			</tbody>
		</table>
	</div>
</MediaQuery>

<MediaQuery query="(max-width: 600px)">
	<div class="device-list">
		{#each devices as device}
			<div class="card">
				<div class="card-title">
					<fluent-text as="h3" size="500" weight="bold" class="device-name"
						>{device.name}</fluent-text
					>
					<fluent-badge class="badge" appearance="filled" color={getBadgeColor(device.status)}>
						{device.status}
					</fluent-badge>
				</div>
				<div>
					<div class="field">
						<div>
							<fluent-text class="field-subtitle">Fingerprint:</fluent-text>
						</div>
						<div>
							<fluent-text font="monospace">{device.fingerprint}</fluent-text>
						</div>
					</div>
					<div class="field">
						<div>
							<fluent-text class="field-subtitle">Last Seen:</fluent-text>
						</div>
						<div>
							<fluent-text>{format(device.lastSeen, 'en_US')}</fluent-text>
						</div>
					</div>
				</div>

				<fluent-menu
					class="card-action"
					onchange={(event: Event) => {
						if (event.target) {
							onUpdateStatus(device.id, (event.target as HTMLElement).getAttribute('value') ?? '');
						}
					}}
				>
					<fluent-button aria-label="Toggle Menu" slot="trigger" class="card-action-trigger">
						Change Permission
					</fluent-button>

					<fluent-menu-list>
						<fluent-menu-item value="approved">Approve</fluent-menu-item>
						<fluent-menu-item value="pending">Pending</fluent-menu-item>
						<fluent-menu-item value="blocked">Block</fluent-menu-item>
					</fluent-menu-list>
				</fluent-menu>
			</div>
		{/each}
	</div>
</MediaQuery>

<style>
	.device-list {
		width: 100%;
		border-collapse: collapse;
	}

	.device-list th {
		padding: 16px;
		text-align: start;
		vertical-align: center;
	}

	.device-list tr {
		border-bottom: 1px solid #e4e4e4;
		transition: background-color 100ms;
	}

	.device-list tr:hover {
		background-color: #fafafa;
	}

	.badge {
		width: 72px;
	}

	.header-text {
		opacity: 0.5;
	}

	.device-list {
		margin-top: 24px;
	}

	.card {
		border: 1px solid #eeeeee;
		margin: 16px 0;
		padding: 16px 24px;
	}

	.card-title {
		display: flex;
		align-items: center;
		justify-content: space-between;
	}

	.card-title .device-name {
		margin-right: 12px;
	}

	.card-action {
		width: 100%;
	}

	.card-action-trigger {
		width: 100%;
		margin-top: 12px;
	}

	.field {
		margin: 12px 0;
	}

	.field-subtitle {
		opacity: 0.5;
	}
</style>
