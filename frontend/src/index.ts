import { faNetworkWired } from '@fortawesome/free-solid-svg-icons';
import type { FC } from 'react';
import { Extension, ExtensionContext } from 'shared';
import { z } from 'zod';
import { type FieldDef, insertFieldsAfter } from '@/elements/form-engine/index.ts';
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

    // 2. Extend admin server forms with splits limit field
    ctx.extensionRegistry.enterForms((forms) => {
      for (const formId of ['admin.servers.create', 'admin.servers.update'] as const) {
        forms.extend(formId, {
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
              type: 'number',
              name: 'featureLimits.splits',
              label: 'Splits Limit',
              description: 'The maximum number of splits allowed for this server. Set to 0 to disable splitting.',
              props: { placeholder: '0', min: 0 },
            } satisfies FieldDef),
        });
      }
    });
  }
}

export default new ComCaloptreyxServerSplitterExtension();
