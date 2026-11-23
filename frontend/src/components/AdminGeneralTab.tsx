import { faSave } from '@fortawesome/free-solid-svg-icons';
import { FontAwesomeIcon } from '@fortawesome/react-fontawesome';
import { Card, Group, NumberInput, Select, SimpleGrid, Stack, Switch, Text, Title } from '@mantine/core';
import { type ReactNode, useEffect, useState } from 'react';
import { httpErrorToHuman } from '@/api/axios.ts';
import Button from '@/elements/buttons/Button.tsx';
import { useToast } from '@/providers/ToastProvider.tsx';
import type { SwapMode } from '../api/client.ts';
import { type AdminSettingsResponse, type SplitterSettings, updateAdminSplitterSettings } from '../api/admin.ts';

const swapModes: { value: SwapMode; label: string }[] = [
  { value: 'pooled', label: "Pooled with the master (memory/4, capped by the master's swap)" },
  { value: 'none', label: 'No swap' },
  { value: 'copy', label: "Copy the master's swap" },
];

function pickSettings(data: AdminSettingsResponse): SplitterSettings {
  return {
    reserved_cpu: data.reserved_cpu,
    reserved_memory: data.reserved_memory,
    reserved_disk: data.reserved_disk,
    minimum_cpu: data.minimum_cpu,
    minimum_memory: data.minimum_memory,
    minimum_disk: data.minimum_disk,
    default_splits: data.default_splits,
    include_disk_usage: data.include_disk_usage,
    display_reserved_limits: data.display_reserved_limits,
    cascade_suspension: data.cascade_suspension,
    cascade_owner: data.cascade_owner,
    mirror_subusers: data.mirror_subusers,
    inherit_pinned_cpus: data.inherit_pinned_cpus,
    swap_mode: data.swap_mode,
  };
}

function SettingsCard({
  title,
  description,
  children,
}: {
  title: string;
  description?: ReactNode;
  children: ReactNode;
}) {
  return (
    <Card padding='lg' radius='md' withBorder>
      <Stack gap='md'>
        <div>
          <Title order={4} fw={600}>
            {title}
          </Title>
          {description && (
            <Text size='xs' c='dimmed'>
              {description}
            </Text>
          )}
        </div>
        {children}
      </Stack>
    </Card>
  );
}

type NumberKey = 'reserved_cpu' | 'reserved_memory' | 'reserved_disk' | 'default_splits';
type MinimumKey = 'minimum_cpu' | 'minimum_memory' | 'minimum_disk';
type SwitchKey =
  | 'include_disk_usage'
  | 'display_reserved_limits'
  | 'cascade_suspension'
  | 'cascade_owner'
  | 'mirror_subusers'
  | 'inherit_pinned_cpus';

const resources: { label: string; unit: string; reserve: NumberKey; minimum: MinimumKey }[] = [
  { label: 'CPU', unit: '%', reserve: 'reserved_cpu', minimum: 'minimum_cpu' },
  { label: 'Memory', unit: 'MB', reserve: 'reserved_memory', minimum: 'minimum_memory' },
  { label: 'Disk', unit: 'MB', reserve: 'reserved_disk', minimum: 'minimum_disk' },
];

const behaviourSwitches: { key: SwitchKey; label: string; description: string }[] = [
  {
    key: 'cascade_suspension',
    label: 'Suspend splits with their master',
    description: 'Unsuspending the master lifts only the suspensions it caused.',
  },
  {
    key: 'cascade_owner',
    label: 'Move splits when the master changes owner',
    description: 'The new owner of a master also gets its splits.',
  },
  {
    key: 'mirror_subusers',
    label: 'Keep synced subusers in sync (permission changes and removals)',
    description: "Changes to a master's subusers reach the splits they were synced to.",
  },
  {
    key: 'inherit_pinned_cpus',
    label: "Splits inherit the master's CPU pinning",
    description: 'New splits run on the same CPU cores as their master.',
  },
];

export default function AdminGeneralTab({
  settings,
  canWrite,
  onSaved,
}: {
  settings: AdminSettingsResponse;
  canWrite: boolean;
  onSaved: () => Promise<void>;
}) {
  const { addToast } = useToast();
  const [form, setForm] = useState<SplitterSettings>(() => pickSettings(settings));
  const [saving, setSaving] = useState(false);

  useEffect(() => {
    setForm(pickSettings(settings));
  }, [settings]);

  const set = <K extends keyof SplitterSettings>(key: K, value: SplitterSettings[K]) =>
    setForm((prev) => ({ ...prev, [key]: value }));

  const handleSave = async () => {
    try {
      setSaving(true);
      await updateAdminSplitterSettings(form);
      addToast('Settings saved', 'success');
      await onSaved();
    } catch (err) {
      addToast(httpErrorToHuman(err), 'error');
    } finally {
      setSaving(false);
    }
  };

  return (
    <Stack gap='md'>
      <SettingsCard
        title='Reserve'
        description='What every master keeps for itself and can never hand out to its splits.'
      >
        <SimpleGrid cols={{ base: 1, sm: 3 }} spacing='md'>
          {resources.map((resource) => (
            <NumberInput
              key={resource.reserve}
              disabled={!canWrite}
              label={`Reserved ${resource.label} (${resource.unit})`}
              min={0}
              value={form[resource.reserve]}
              onChange={(val) => set(resource.reserve, typeof val === 'number' ? val : 0)}
            />
          ))}
        </SimpleGrid>
      </SettingsCard>

      <SettingsCard
        title='Minimum split size'
        description='The smallest split users may create or resize to. Leave empty to use the reserve.'
      >
        <SimpleGrid cols={{ base: 1, sm: 3 }} spacing='md'>
          {resources.map((resource) => (
            <NumberInput
              key={resource.minimum}
              disabled={!canWrite}
              label={`Minimum ${resource.label} (${resource.unit})`}
              min={0}
              placeholder={`${form[resource.reserve]} (same as the reserve)`}
              value={form[resource.minimum] ?? ''}
              onChange={(val) => set(resource.minimum, typeof val === 'number' ? val : null)}
            />
          ))}
        </SimpleGrid>
      </SettingsCard>

      <SimpleGrid cols={{ base: 1, md: 2 }} spacing='md'>
        <SettingsCard title='Split behaviour'>
          {behaviourSwitches.map((item) => (
            <Switch
              key={item.key}
              disabled={!canWrite}
              label={item.label}
              description={item.description}
              checked={form[item.key]}
              onChange={(e) => set(item.key, e.currentTarget.checked)}
            />
          ))}
          <Select
            disabled={!canWrite}
            label='Swap for splits'
            description='Users never set swap; the splitter sets it from this rule.'
            data={swapModes}
            allowDeselect={false}
            value={form.swap_mode}
            onChange={(val) => val && set('swap_mode', val as SwapMode)}
          />
        </SettingsCard>

        <Stack gap='md'>
          <SettingsCard title='New servers'>
            <NumberInput
              disabled={!canWrite}
              label='Default split limit'
              description='Split limit given to new servers unless set on the server.'
              min={0}
              value={form.default_splits}
              onChange={(val) => set('default_splits', typeof val === 'number' ? val : 0)}
            />
          </SettingsCard>

          <SettingsCard title='Display'>
            <Switch
              disabled={!canWrite}
              label="Count the master's live disk usage"
              description='Disk the master already uses is taken off the disk that can still be split.'
              checked={form.include_disk_usage}
              onChange={(e) => set('include_disk_usage', e.currentTarget.checked)}
            />
            <Switch
              disabled={!canWrite}
              label='Show the reserve to users'
              description='When off, the reserve is counted as free in the display only; users still cannot take it.'
              checked={form.display_reserved_limits}
              onChange={(e) => set('display_reserved_limits', e.currentTarget.checked)}
            />
          </SettingsCard>
        </Stack>
      </SimpleGrid>

      {canWrite && (
        <Group justify='flex-end'>
          <Button color='blue' loading={saving} onClick={handleSave} leftSection={<FontAwesomeIcon icon={faSave} />}>
            Save Settings
          </Button>
        </Group>
      )}
    </Stack>
  );
}
