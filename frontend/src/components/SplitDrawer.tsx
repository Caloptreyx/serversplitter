import { faChevronDown, faChevronRight, faCircleInfo, faTriangleExclamation } from '@fortawesome/free-solid-svg-icons';
import { FontAwesomeIcon } from '@fortawesome/react-fontawesome';
import { Slider, UnstyledButton } from '@mantine/core';
import { useMediaQuery } from '@mantine/hooks';
import { type ReactNode, useEffect, useMemo, useState } from 'react';
import { httpErrorToHuman } from '@/api/axios.ts';
import Button from '@/elements/buttons/Button.tsx';
import Alert from '@/elements/feedback/Alert.tsx';
import NumberInput from '@/elements/input/NumberInput.tsx';
import Select from '@/elements/input/Select.tsx';
import Switch from '@/elements/input/Switch.tsx';
import TextArea from '@/elements/input/TextArea.tsx';
import TextInput from '@/elements/input/TextInput.tsx';
import Collapse from '@/elements/layout/Collapse.tsx';
import SegmentedControl from '@/elements/layout/SegmentedControl.tsx';
import Drawer from '@/elements/overlays/Drawer.tsx';
import { useServerCan } from '@/plugins/usePermissions.ts';
import { useToast } from '@/providers/ToastProvider.tsx';
import { useServerStore } from '@/stores/server.ts';
import {
  createSplit,
  type FeatureLimitsInput,
  getClientSplitterNests,
  type MasterPower,
  type MasterPowerOptions,
  type NestEggItem,
  type Pool,
  type SplitEntry,
  type SplitterPort,
  updateSplit,
} from '../api/client.ts';
import MasterPowerSelect, { initialMasterPower, masterPowerOffered, reportMasterPower } from './MasterPowerSelect.tsx';
import { formatDelta, formatResource, type PoolResourceKey, RESOURCE_LABELS } from './splitterFormat.ts';

/** `max` null = no upper bound. */
interface Bound {
  min: number;
  max: number | null;
}

type Resources = Record<PoolResourceKey, number>;
type FeatureKey = keyof FeatureLimitsInput;

const RESOURCE_KEYS: PoolResourceKey[] = ['cpu', 'memory', 'disk'];
const FEATURE_KEYS: FeatureKey[] = ['allocations', 'databases', 'backups', 'schedules'];
const FEATURE_LABELS: Record<FeatureKey, string> = {
  allocations: 'Ports',
  databases: 'Databases',
  backups: 'Backups',
  schedules: 'Schedules',
};
const DEFAULT_RESOURCES: Resources = { cpu: 100, memory: 1024, disk: 2048 };

const unbounded = (max: number): number | null => (max === -1 ? null : max);
const inBound = (value: number, { min, max }: Bound) => value >= min && (max === null || value <= max);
const portLabel = (port: SplitterPort) => `${port.ip_alias || port.ip}:${port.port}`;

function createBound(pool: Pool, key: PoolResourceKey): Bound {
  const res = pool[key];
  // 0 (= unlimited) is only allowed where the master itself is unlimited.
  const min = res.master === 0 ? pool.minimum[key] : Math.max(pool.minimum[key], 1);
  return { min, max: unbounded(res.free) };
}

const eggDefaults = (egg: NestEggItem | null): Record<string, string> =>
  Object.fromEntries((egg?.variables ?? []).map((v) => [v.env_variable, v.default_value ?? '']));

function ResourceField({
  resource,
  value,
  bound,
  onChange,
}: {
  resource: PoolResourceKey;
  value: number;
  bound: Bound;
  onChange: (value: number) => void;
}) {
  const isCpu = resource === 'cpu';
  // memory and disk are entered in GB and kept as MB
  const scale = isCpu ? 1 : 1024;
  const format = (v: number) => formatResource(resource, v);
  const error = !inBound(value, bound)
    ? bound.max === null
      ? `At least ${format(bound.min)}`
      : `Between ${format(bound.min)} and ${format(bound.max)}`
    : undefined;

  return (
    <div className='grid grid-cols-[4rem_1fr_6.5rem] sm:grid-cols-[4.5rem_1fr_7.5rem] items-center gap-3'>
      <span className='font-medium'>{RESOURCE_LABELS[resource]}</span>
      {bound.max !== null && bound.max > bound.min ? (
        <div className='flex flex-col gap-1'>
          <Slider
            min={bound.min}
            max={bound.max}
            step={isCpu ? 1 : 256}
            value={Math.min(Math.max(value, bound.min), bound.max)}
            onChange={onChange}
            label={format}
            aria-label={RESOURCE_LABELS[resource]}
          />
          <span className='text-xs text-(--mantine-color-dimmed) text-right'>max {format(bound.max)}</span>
        </div>
      ) : (
        <span className='text-xs text-(--mantine-color-dimmed)'>
          {bound.max !== null
            ? `Fixed at ${format(bound.max)}`
            : bound.min === 0
              ? 'No limit on the pool, 0 = unlimited'
              : `No upper limit, at least ${format(bound.min)}`}
        </span>
      )}
      <NumberInput
        aria-label={RESOURCE_LABELS[resource]}
        value={value / scale}
        min={bound.min / scale}
        max={bound.max === null ? undefined : bound.max / scale}
        step={isCpu ? 1 : 0.25}
        decimalScale={isCpu ? 0 : 2}
        allowNegative={false}
        clampBehavior='blur'
        rightSection={<span className='text-xs text-(--mantine-color-dimmed)'>{isCpu ? '%' : 'GB'}</span>}
        error={error}
        onChange={(v) => {
          if (typeof v === 'number') onChange(Math.round(v * scale));
        }}
      />
    </div>
  );
}

function Section({
  title,
  summary,
  opened,
  onToggle,
  children,
}: {
  title: string;
  summary?: string;
  opened: boolean;
  onToggle: () => void;
  children: ReactNode;
}) {
  return (
    <div className='rounded-md border border-(--mantine-color-default-border)'>
      <UnstyledButton
        className='flex w-full items-center gap-2 px-3 py-2 text-sm'
        onClick={onToggle}
        aria-expanded={opened}
      >
        <FontAwesomeIcon icon={opened ? faChevronDown : faChevronRight} className='size-3' />
        <span className='font-medium shrink-0'>{title}</span>
        {summary && <span className='truncate min-w-0 text-(--mantine-color-dimmed)'>· {summary}</span>}
      </UnstyledButton>
      <Collapse expanded={opened}>
        <div className='flex flex-col gap-3 px-3 pb-3'>{children}</div>
      </Collapse>
    </div>
  );
}

export default function SplitDrawer({
  opened,
  onClose,
  onSaved,
  serverUuid,
  masterName,
  pool,
  split,
  masterPower,
}: {
  opened: boolean;
  onClose: () => void;
  onSaved: () => void;
  serverUuid: string;
  masterName: string;
  pool: Pool;
  /** Resize this split; `null` creates a new one. */
  split: SplitEntry | null;
  /** Power actions offered after the change (`null` = none). */
  masterPower: MasterPowerOptions | null;
}) {
  const { addToast } = useToast();
  const isPhone = useMediaQuery('(max-width: 48em)');
  const canStart = useServerCan('control.start');
  const canCreateSubusers = useServerCan('subusers.create');
  const masterState = useServerStore((s) => s.state);

  const [nests, setNests] = useState<Record<string, NestEggItem[]> | null>(null);
  const [name, setName] = useState('');
  const [description, setDescription] = useState('');
  const [eggUuid, setEggUuid] = useState<string | null>(null);
  const [resources, setResources] = useState<Resources>(DEFAULT_RESOURCES);
  const [features, setFeatures] = useState<FeatureLimitsInput>({
    allocations: 1,
    databases: 0,
    backups: 0,
    schedules: 0,
  });
  const [portType, setPortType] = useState<'master' | 'node'>('node');
  const [masterPort, setMasterPort] = useState<string | null>(null);
  const [variables, setVariables] = useState<Record<string, string>>({});
  const [syncSubusers, setSyncSubusers] = useState(false);
  const [featuresOpen, setFeaturesOpen] = useState(false);
  const [variablesOpen, setVariablesOpen] = useState(false);
  const [submitting, setSubmitting] = useState(false);
  const [power, setPower] = useState<MasterPower>('none');
  const powerOffered = masterPowerOffered(masterPower, masterState);

  const transferable = pool.transferable_ports;

  const resourceBounds = useMemo(
    () =>
      Object.fromEntries(
        RESOURCE_KEYS.map((key) => [
          key,
          split ? { min: split.min[key], max: unbounded(split.max[key]) } : createBound(pool, key),
        ]),
      ) as Record<PoolResourceKey, Bound>,
    [pool, split],
  );

  const featureBounds: Record<FeatureKey, Bound> = split
    ? (Object.fromEntries(
        FEATURE_KEYS.map((key) => [key, { min: split.min[key], max: unbounded(split.max[key]) }]),
      ) as Record<FeatureKey, Bound>)
    : {
        allocations: {
          min: 1,
          max: portType === 'master' ? pool.allocations_free_with_master_port : pool.allocations.free,
        },
        databases: { min: 0, max: pool.databases.free },
        backups: { min: 0, max: pool.backups.free },
        schedules: { min: 0, max: pool.schedules.free },
      };

  const eggs = useMemo(() => Object.values(nests ?? {}).flat(), [nests]);
  const egg = eggs.find((e) => e.uuid === eggUuid) ?? null;

  // Reset the form every time the drawer opens.
  useEffect(() => {
    if (!opened) return;

    setSubmitting(false);
    setFeaturesOpen(false);
    setVariablesOpen(false);
    setPower(initialMasterPower(masterPower));

    if (split) {
      setName(split.server.name);
      setDescription(split.server.description ?? '');
      setResources({ cpu: split.ledger.cpu, memory: split.ledger.memory, disk: split.ledger.disk });
      setFeatures({
        allocations: split.ledger.allocations,
        databases: split.ledger.databases,
        backups: split.ledger.backups,
        schedules: split.ledger.schedules,
      });
      return;
    }

    setName('');
    setDescription('');
    setResources(
      Object.fromEntries(
        RESOURCE_KEYS.map((key) => {
          const { min, max } = resourceBounds[key];
          return [key, Math.max(min, max === null ? DEFAULT_RESOURCES[key] : Math.min(DEFAULT_RESOURCES[key], max))];
        }),
      ) as Resources,
    );
    setFeatures({ allocations: 1, databases: 0, backups: 0, schedules: 0 });
    setPortType(transferable.length > 0 ? 'master' : 'node');
    setMasterPort(transferable[0]?.uuid ?? null);
    setSyncSubusers(canCreateSubusers);
    setVariables(eggDefaults(egg));

    if (nests === null) {
      getClientSplitterNests(serverUuid)
        .then(setNests)
        .catch((err) => {
          setNests({});
          addToast(httpErrorToHuman(err), 'error');
        });
    }
  }, [opened]);

  // Pick the first egg once the nests are there, and prefill its variables.
  useEffect(() => {
    if (!eggUuid && eggs.length > 0) setEggUuid(eggs[0].uuid);
  }, [eggs]);

  useEffect(() => {
    setVariables(eggDefaults(egg));
  }, [egg?.uuid]);

  const eggOptions = useMemo(
    () =>
      Object.entries(nests ?? {}).map(([nest, nestEggs]) => ({
        group: nest,
        items: nestEggs.map((e) => ({ value: e.uuid, label: e.name })),
      })),
    [nests],
  );

  const trimmedName = name.trim();
  const nameError =
    trimmedName.length > 0 && (trimmedName.length < 3 || trimmedName.length > 255) ? '3 to 255 characters' : undefined;
  const tooSmall = RESOURCE_KEYS.filter((key) => {
    const { min, max } = resourceBounds[key];
    return max !== null && max < min;
  });
  const valid =
    trimmedName.length >= 3 &&
    trimmedName.length <= 255 &&
    (split !== null || (eggUuid !== null && (portType === 'node' || masterPort !== null))) &&
    RESOURCE_KEYS.every((key) => inBound(resources[key], resourceBounds[key])) &&
    FEATURE_KEYS.every((key) => inBound(features[key], featureBounds[key]));
  const outOfBounds = [
    ...RESOURCE_KEYS.filter((key) => !inBound(resources[key], resourceBounds[key])),
    ...FEATURE_KEYS.filter((key) => !inBound(features[key], featureBounds[key])).map(
      (key) => `${FEATURE_LABELS[key].toLowerCase()} (${featureBounds[key].min} to ${featureBounds[key].max ?? '∞'})`,
    ),
  ];

  const masterKeeps = RESOURCE_KEYS.map((key) => {
    const master = pool[key].master;
    if (master === 0) return 'Unlimited';
    const taken = split ? resources[key] - split.ledger[key] : resources[key];
    return formatResource(key, master - taken);
  }).join(' · ');

  const submit = async () => {
    if (!valid) return;
    setSubmitting(true);

    try {
      const masterPowerChoice = powerOffered ? power : 'none';
      if (split) {
        const result = await updateSplit(serverUuid, split.server.uuid, {
          name: trimmedName,
          // always sent: an empty string clears it
          description: description.trim(),
          cpu: resources.cpu,
          memory: resources.memory,
          disk: resources.disk,
          feature_limits: features,
          master_power: masterPowerChoice,
        });
        reportMasterPower(addToast, `${trimmedName} resized`, result.masterPower);
      } else {
        const changed = (egg?.variables ?? [])
          .filter((v) => (variables[v.env_variable] ?? '') !== (v.default_value ?? ''))
          .map((v) => ({ env_variable: v.env_variable, value: variables[v.env_variable] ?? '' }));

        const result = await createSplit(serverUuid, {
          name: trimmedName,
          description: description.trim() || undefined,
          egg_uuid: eggUuid!,
          cpu: resources.cpu,
          memory: resources.memory,
          disk: resources.disk,
          feature_limits: features,
          port:
            portType === 'master' && masterPort ? { type: 'master', allocation_uuid: masterPort } : { type: 'node' },
          variables: changed.length > 0 ? changed : undefined,
          sync_subusers: canCreateSubusers && syncSubusers,
          master_power: masterPowerChoice,
        });
        reportMasterPower(addToast, `${trimmedName} created`, result.masterPower);
      }

      onClose();
      onSaved();
    } catch (err) {
      addToast(httpErrorToHuman(err), 'error');
    } finally {
      setSubmitting(false);
    }
  };

  const firstPort = transferable.length === 1 ? portLabel(transferable[0]) : null;

  return (
    <Drawer
      opened={opened}
      onClose={onClose}
      position={isPhone ? 'bottom' : 'right'}
      size={isPhone ? '100%' : 480}
      title={
        <div>
          <p className='text-lg font-semibold'>{split ? `Resize ${split.server.name}` : 'New split'}</p>
          <p className='text-sm text-(--mantine-color-dimmed)'>
            {split
              ? `Resources go back to or come from ${masterName}.`
              : `Carved out of ${masterName}. You can resize it later.`}
          </p>
        </div>
      }
    >
      <div className='flex flex-col gap-4'>
        <TextInput
          label='Name'
          required
          maxLength={255}
          value={name}
          error={nameError}
          onChange={(e) => setName(e.currentTarget.value)}
        />
        <TextArea
          label='Description'
          autosize
          minRows={1}
          maxLength={1024}
          value={description}
          onChange={(e) => setDescription(e.currentTarget.value)}
        />

        {!split && (
          <div className='flex flex-col gap-1'>
            <Select
              label='Egg'
              required
              searchable
              data={eggOptions}
              value={eggUuid}
              onChange={setEggUuid}
              disabled={nests === null}
              placeholder={nests === null ? 'Loading eggs...' : 'Select egg'}
            />
            {egg?.description && <p className='text-xs text-(--mantine-color-dimmed)'>{egg.description}</p>}
            {nests !== null && eggs.length === 0 && (
              <Alert color='yellow' icon={faTriangleExclamation}>
                No eggs are allowed for splitting. Contact an administrator.
              </Alert>
            )}
          </div>
        )}

        <div className='flex flex-col gap-3'>
          <p className='text-sm'>
            <span className='font-medium'>Resources</span>
            <span className='text-(--mantine-color-dimmed)'> · drag or type</span>
          </p>
          {tooSmall.length > 0 && (
            <Alert color='yellow' icon={faTriangleExclamation}>
              Not enough free {tooSmall.map((key) => RESOURCE_LABELS[key].toLowerCase()).join(', ')} on {masterName} for
              a split.
            </Alert>
          )}
          {RESOURCE_KEYS.map((key) => (
            <ResourceField
              key={key}
              resource={key}
              value={resources[key]}
              bound={resourceBounds[key]}
              onChange={(value) => setResources((prev) => ({ ...prev, [key]: value }))}
            />
          ))}
        </div>

        {!split && (
          <div className='flex flex-col gap-2'>
            <span className='text-sm font-medium'>Port</span>
            <SegmentedControl
              fullWidth
              orientation={isPhone ? 'vertical' : 'horizontal'}
              value={portType}
              onChange={(value) => setPortType(value as 'master' | 'node')}
              data={[
                {
                  value: 'master',
                  label:
                    transferable.length > 1 ? 'A spare port' : firstPort ? `Spare port ${firstPort}` : 'A spare port',
                  disabled: transferable.length === 0,
                },
                { value: 'node', label: 'New port' },
              ]}
            />
            {portType === 'master' && transferable.length > 1 && (
              <Select
                aria-label='Port to take'
                data={transferable.map((port) => ({
                  value: port.uuid,
                  label: port.notes ? `${portLabel(port)} (${port.notes})` : portLabel(port),
                }))}
                value={masterPort}
                onChange={setMasterPort}
              />
            )}
            {transferable.length === 0 && (
              <p className='text-xs text-(--mantine-color-dimmed)'>
                {masterName} has no spare port to give away, the split gets a new one.
              </p>
            )}
          </div>
        )}

        <Section
          title='Feature limits'
          summary={FEATURE_KEYS.map((key) => `${FEATURE_LABELS[key]} ${features[key]}`).join(' · ')}
          opened={featuresOpen}
          onToggle={() => setFeaturesOpen((o) => !o)}
        >
          <div className='grid grid-cols-2 gap-3'>
            {FEATURE_KEYS.map((key) => {
              const { min, max } = featureBounds[key];
              return (
                <NumberInput
                  key={key}
                  label={FEATURE_LABELS[key]}
                  description={max === null ? 'No upper limit' : `Max ${max}`}
                  min={min}
                  max={max ?? undefined}
                  allowDecimal={false}
                  allowNegative={false}
                  clampBehavior='blur'
                  value={features[key]}
                  error={!inBound(features[key], featureBounds[key]) ? `${min} to ${max ?? '∞'}` : undefined}
                  onChange={(v) => setFeatures((prev) => ({ ...prev, [key]: typeof v === 'number' ? v : min }))}
                />
              );
            })}
          </div>
        </Section>

        {!split && egg && egg.variables.length > 0 && (
          <Section
            title='Startup variables'
            summary={`${egg.variables.length} from ${egg.name}`}
            opened={variablesOpen}
            onToggle={() => setVariablesOpen((o) => !o)}
          >
            {egg.variables.map((variable) => (
              <TextInput
                key={variable.env_variable}
                label={variable.name}
                description={variable.description || variable.env_variable}
                value={variables[variable.env_variable] ?? ''}
                onChange={(e) => {
                  const value = e.currentTarget.value;
                  setVariables((prev) => ({ ...prev, [variable.env_variable]: value }));
                }}
              />
            ))}
          </Section>
        )}

        {!split && (
          <Switch
            label='Copy subusers from master'
            description={
              canCreateSubusers
                ? `${masterName}'s subusers get the same permissions on the split.`
                : 'You need the permission to create subusers for this.'
            }
            disabled={!canCreateSubusers}
            checked={canCreateSubusers && syncSubusers}
            onChange={(e) => setSyncSubusers(e.currentTarget.checked)}
          />
        )}

        <div className='rounded-md border border-(--mantine-color-blue-outline) bg-(--mantine-color-blue-light) px-3 py-2 text-sm'>
          {split && (
            <p>
              Change for <span className='font-semibold'>{split.server.name}</span>:{' '}
              {RESOURCE_KEYS.map((key) => formatDelta(key, resources[key] - split.ledger[key])).join(' · ')}
            </p>
          )}
          <p>
            After this {split ? 'resize' : 'split'} <span className='font-semibold'>{masterName}</span> keeps:
          </p>
          <p className='font-semibold'>
            {masterKeeps}
            {pool.display_reserved && RESOURCE_KEYS.some((key) => pool[key].reserved > 0) && (
              <span className='font-normal text-(--mantine-color-dimmed)'> (incl. reserve)</span>
            )}
          </p>
        </div>

        {!split && (
          <p className='flex items-start gap-2 text-xs text-(--mantine-color-dimmed)'>
            <FontAwesomeIcon icon={faCircleInfo} className='mt-0.5' />
            {canStart
              ? 'The split installs and starts by itself.'
              : "The split installs by itself but won't start by itself; you need the Start permission."}
          </p>
        )}

        {outOfBounds.length > 0 && (
          <p className='text-xs text-(--mantine-color-red-text)'>
            Out of range: {outOfBounds.join(', ')}.
            {!split && portType === 'node' && pool.allocations.free < 1 && transferable.length > 0
              ? ` ${masterName} has no free port slot; take one of its spare ports instead.`
              : ''}
          </p>
        )}

        {powerOffered && masterPower && <MasterPowerSelect options={masterPower} value={power} onChange={setPower} />}

        <div className='flex justify-end gap-2'>
          <Button variant='default' onClick={onClose}>
            Cancel
          </Button>
          <Button loading={submitting} disabled={!valid || tooSmall.length > 0} onClick={submit}>
            {split ? 'Save' : 'Create split'}
          </Button>
        </div>
      </div>
    </Drawer>
  );
}
