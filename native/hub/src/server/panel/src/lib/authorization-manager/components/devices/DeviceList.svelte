<script lang="ts">
	import { format } from 'timeago.js';

	type Device = {
		id: string;
		name: string;
		fingerprint: string;
		status: string;
		lastSeen: Date;
	};

	type Props = {
		devices: Device[];
		onUpdateStatus: (deviceId: string, status: string) => void;
	};

	let { devices, onUpdateStatus }: Props = $props();

	/** Get badge variant based on status */
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

<!-- Desktop view - Table -->
<div class="hidden md:block">
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
					<th><fluent-text font="monospace" truncate="true">{device.fingerprint}</fluent-text></th>
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

<!-- Mobile view - Cards -->
<div class="md:hidden">
	{#each devices as device}
		<div>
			<div>
				<div class="card-title">
					<fluent-text as="h3" size="500" weight="bold" class="device-name"
						>{device.name}</fluent-text
					>
					<fluent-badge appearance="filled" color={getBadgeColor(device.status)}>
						{device.status}
					</fluent-badge>
				</div>
				<div>
					<div>
						<div>
							<fluent-text weight="semibold">Fingerprint:</fluent-text>
						</div>
						<div>
							<fluent-text font="monospace">{device.fingerprint}</fluent-text>
						</div>
					</div>
					<div>
						<div>
							<fluent-text weight="semibold">Last Seen:</fluent-text>
						</div>
						<div>
							<fluent-text>{format(device.lastSeen, 'en_US')}</fluent-text>
						</div>
					</div>
				</div>
				<fluent-dropdown
					placeholder="Status"
					class="dropdown"
					appearance="filled-lighter"
					onchange={(event: Event) => {
						if (event.target) {
							onUpdateStatus(device.id, (event.target as HTMLSelectElement).value);
						}
					}}
				>
					<fluent-listbox>
						<fluent-option value="approved">Approve</fluent-option>
						<fluent-option value="pending">Pending</fluent-option>
						<fluent-option value="blocked">Block</fluent-option>
					</fluent-listbox>
				</fluent-dropdown>
			</div>
		</div>
	{/each}
</div>

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

	.card-title {
		display: flex;
		align-items: center;
	}

	.card-title .device-name {
		margin-right: 12px;
	}
</style>
