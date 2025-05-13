import { default as Authorizationmanager } from './App.svelte';

export interface IDevice {
	id: string;
	name: string;
	fingerprint: string;
	status: string;
	lastSeen: Date;
}

export interface IServerConfig {
	alias: string;
	broadcastEnabled: boolean;
	fingerprint: string;
}

export default Authorizationmanager;

export type DeviceType = 'Mobile' | 'Desktop' | 'Web' | 'Headless' | 'Server' | 'Unknown';

export type UserStatus = 'Approved' | 'Pending' | 'Blocked';

export interface IUserSummaryResponse {
	alias: string;
	fingerprint: string;
	device_model: string;
	device_type: DeviceType;
	status: UserStatus;
	add_time: {
		secs_since_epoch: number;
		nanos_since_epoch: number;
	};
}
