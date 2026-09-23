import { faNetworkWired } from '@fortawesome/free-solid-svg-icons';
import type { FC } from 'react';
import { Extension, ExtensionContext } from 'shared';
import { z } from 'zod';
import { type FieldDef, insertFieldsAfter } from '@/elements/form-engine/index.ts';
import { nullableNumber } from '@/lib/serialization/transformers.ts';
import AdminServerSplitterPage from './pages/AdminServerSplitterPage.tsx';
import ServerSplitterPage from './pages/ServerSplitterPage.tsx';

class ComCaloptreyxServerSplitterExtension extends Extension {
  public cardConfigurationPage: FC | null = AdminServerSplitterPage;
  public cardComponent: FC | null = null;

  public initialize(ctx: ExtensionContext): void {
    // 1. Client Server Splitter Tab
    ctx.extensionRegistry.routes.addServerRoute({
      name: 'Splitter',
      icon: faNetworkWired,
      path: '/splitter',
      exact: true,
      permission: 'splitter.read',
      element: ServerSplitterPage,
    });

    // 2. Extend admin server forms with the splits limit field
    ctx.extensionRegistry.enterForms((forms) => {
      const splitsField = {
        type: 'number',
        name: 'featureLimits.splits',
        label: 'Splits Limit',
      } as const;

      // create: left empty, the server gets the Default Split Limit from the Server Splitter settings
      forms.extend('admin.servers.create', {
        zodShape: {
          featureLimits: z.object({
            splits: z.preprocess(nullableNumber, z.number().int().min(0).nullable()),
          }),
        },
        initialValues: {
          featureLimits: {
            splits: null,
          },
        },
        transform: (fields) =>
          insertFieldsAfter(fields, 'featureLimits.schedules', {
            ...splitsField,
            description: 'Maximum number of splits for this server. Leave empty to use the default split limit.',
            props: { placeholder: 'Default', min: 0 },
          } satisfies FieldDef),
      });

      forms.extend('admin.servers.update', {
        zodShape: {
          featureLimits: z.object({
            splits: z.coerce.number().int().min(0).default(0),
          }),
        },
        initialValues: {
          featureLimits: {
            splits: 0,
          },
        },
        transform: (fields) =>
          insertFieldsAfter(fields, 'featureLimits.schedules', {
            ...splitsField,
            description: 'The maximum number of splits allowed for this server. Set to 0 to disable splitting.',
            props: { placeholder: '0', min: 0 },
          } satisfies FieldDef),
      });
    });
  }
}

export default new ComCaloptreyxServerSplitterExtension();
