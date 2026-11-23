import { faDiscord } from '@fortawesome/free-brands-svg-icons';
import { faCog, faEgg, faNetworkWired } from '@fortawesome/free-solid-svg-icons';
import { FontAwesomeIcon } from '@fortawesome/react-fontawesome';
import { Badge, Group, Button as MantineButton, Stack, Tabs, Text, Title } from '@mantine/core';
import { useEffect, useState } from 'react';
import { httpErrorToHuman } from '@/api/axios.ts';
import AlertError from '@/elements/alerts/AlertError.tsx';
import Button from '@/elements/buttons/Button.tsx';
import Spinner from '@/elements/feedback/Spinner.tsx';
import { useAdminCan } from '@/plugins/usePermissions.ts';
import { useToast } from '@/providers/ToastProvider.tsx';
import { type AdminSettingsResponse, getAdminSplitterSettings } from '../api/admin.ts';
import AdminEggRulesTab from '../components/AdminEggRulesTab.tsx';
import AdminGeneralTab from '../components/AdminGeneralTab.tsx';
import AdminMastersTab from '../components/AdminMastersTab.tsx';

export default function AdminServerSplitterPage() {
  const { addToast } = useToast();
  const canWrite = useAdminCan('extensions.splitter.write');

  const [loading, setLoading] = useState(true);
  const [settings, setSettings] = useState<AdminSettingsResponse | null>(null);
  const [loadError, setLoadError] = useState('');

  const loadSettings = async () => {
    try {
      setLoading(true);
      setLoadError('');
      setSettings(await getAdminSplitterSettings());
    } catch (err) {
      setLoadError(httpErrorToHuman(err));
      addToast(httpErrorToHuman(err), 'error');
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    loadSettings();
  }, []);

  const header = (
    <Group justify='space-between' align='center'>
      <div>
        <Title order={3} fw={700}>
          Server Splitter Configuration
        </Title>
        <Text size='sm' c='dimmed'>
          Configure master reserves, split behaviour and egg rules, and check every master&apos;s pool.
        </Text>
      </div>
      <MantineButton
        component='a'
        href='https://discord.gg/4qjMWU7S8x'
        target='_blank'
        rel='noopener noreferrer'
        variant='default'
        leftSection={<FontAwesomeIcon icon={faDiscord} />}
      >
        Support & feature requests
      </MantineButton>
    </Group>
  );

  if (loading && !settings) {
    return (
      <div className='flex justify-center items-center py-24'>
        <Spinner />
      </div>
    );
  }

  if (!settings) {
    // Never render the editable form with placeholder values: saving it would overwrite real settings.
    return (
      <Stack gap='lg'>
        {header}
        {loadError && <AlertError error={loadError} setError={setLoadError} />}
        <Group>
          <Button onClick={loadSettings}>Retry</Button>
        </Group>
      </Stack>
    );
  }

  return (
    <Stack gap='lg'>
      {header}

      <Tabs defaultValue='general'>
        <Tabs.List mb='md'>
          <Tabs.Tab value='general' leftSection={<FontAwesomeIcon icon={faCog} />}>
            General
          </Tabs.Tab>
          <Tabs.Tab
            value='eggRules'
            leftSection={<FontAwesomeIcon icon={faEgg} />}
            rightSection={
              <Badge size='xs' variant='filled'>
                {settings.egg_rules.length}
              </Badge>
            }
          >
            Egg Rules
          </Tabs.Tab>
          <Tabs.Tab value='splits' leftSection={<FontAwesomeIcon icon={faNetworkWired} />}>
            Splits
          </Tabs.Tab>
        </Tabs.List>

        <Tabs.Panel value='general'>
          <AdminGeneralTab settings={settings} canWrite={canWrite} onSaved={loadSettings} />
        </Tabs.Panel>

        <Tabs.Panel value='eggRules'>
          <AdminEggRulesTab settings={settings} canWrite={canWrite} onChanged={loadSettings} />
        </Tabs.Panel>

        {/* Loads the masters list only once the tab is opened. */}
        <Tabs.Panel value='splits' keepMounted={false}>
          <AdminMastersTab canWrite={canWrite} />
        </Tabs.Panel>
      </Tabs>
    </Stack>
  );
}
