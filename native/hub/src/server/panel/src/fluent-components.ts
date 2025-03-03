import { provideFluentDesignSystem, fluentSelect, fluentOption } from '@fluentui/web-components';

export const registerFluentComponents = () => {
	provideFluentDesignSystem().register(fluentSelect(), fluentOption());

	console.log('REGISTERED');
};
