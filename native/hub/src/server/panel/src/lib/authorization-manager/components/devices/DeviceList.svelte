<script lang="ts">
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
	<table>
		<thead>
			<tr>
				<th>Device Name</th>
				<th>Fingerprint</th>
				<th>Status</th>
				<th>Last Seen</th>
				<th>Actions</th>
			</tr>
		</thead>
		<tbody>
			{#each devices as device}
				<tr>
					<th>{device.name}</th>
					<th class="font-mono">{device.fingerprint}</th>
					<th>
						<fluent-badge appearance="filled" color={getBadgeColor(device.status)}>
							{device.status}
						</fluent-badge>
					</th>
					<th>{device.lastSeen.toLocaleString()}</th>
					<th>
						<fluent-dropdown
							placeholder="Change Status"
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
					</th>
				</tr>
			{/each}
		</tbody>
	</table>
</div>

<!-- Mobile view - Cards -->
<div class="grid gap-4 md:hidden">
	{#each devices as device}
		<ax-elevation class="pt-6" style="--elevation-depth: 2">
			<div class="space-y-4">
				<div class="flex items-center justify-between">
					<h3 class="font-semibold">{device.name}</h3>
					<fluent-badge appearance="filled" color={getBadgeColor(device.status)}>
						{device.status}
					</fluent-badge>
				</div>
				<div class="space-y-2 text-sm">
					<div class="flex flex-col gap-1">
						<span class="text-muted-foreground">Fingerprint:</span>
						<code class="bg-muted rounded px-2 py-1">{device.fingerprint}</code>
					</div>
					<div>
						<span class="text-muted-foreground">Last Seen:</span>
						<p>{device.lastSeen.toLocaleString()}</p>
					</div>
				</div>
				<fluent-dropdown
					title="Change Status"
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
		</ax-elevation>
	{/each}
</div>
