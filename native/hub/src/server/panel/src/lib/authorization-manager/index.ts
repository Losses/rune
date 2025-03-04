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
}

export default Authorizationmanager;
