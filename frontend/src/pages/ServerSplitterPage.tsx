import {
  faArrowRight,
  faEdit,
  faExclamationTriangle,
  faHdd,
  faInfoCircle,
  faMemory,
  faMicrochip,
  faNetworkWired,
  faPlus,
  faServer,
  faTrash,
  faUsers,
} from '@fortawesome/free-solid-svg-icons';
import { FontAwesomeIcon } from '@fortawesome/react-fontawesome';
import {
  Alert,
  Badge,
  Card,
  Group,
  NumberInput,
  Progress,
  Select,
  SimpleGrid,
  Stack,
  Switch,
  Text,
  TextInput,
  Title,
  Tooltip,
} from '@mantine/core';
import { useEffect, useMemo, useState } from 'react';
import { NavLink as Link } from 'react-router';
import { httpErrorToHuman } from '@/api/axios.ts';
import Button from '@/elements/buttons/Button.tsx';
import ServerContentContainer from '@/elements/containers/ServerContentContainer.tsx';
import Spinner from '@/elements/feedback/Spinner.tsx';
import ConfirmationModal from '@/elements/modals/ConfirmationModal.tsx';
import { Modal, ModalFooter } from '@/elements/modals/Modal.tsx';
import { useToast } from '@/providers/ToastProvider.tsx';
import { useServerStore } from '@/stores/server.ts';
import {
  createSplit,
  deleteSplit,
  getClientSplitter,
  getClientSplitterNests,
  type NestEggItem,
  type Server,
  type SplitterClientIndex,
  syncSubusers,
  updateSplit,
} from '../api/client.ts';

function formatBytes(mb: number): string {
  if (mb >= 1024) {
    return `${(mb / 1024).toFixed(1)} GB`;
  }
  return `${mb} MB`;
}

export default function ServerSplitterPage() {
  const currentServer = useServerStore((state) => state.server);
  const { addToast } = useToast();

  const [loading, setLoading] = useState(true);
  const [data, setData] = useState<SplitterClientIndex | null>(null);
  const [nestsData, setNestsData] = useState<Record<string, NestEggItem[]>>({});

  // Modals state
  const [isCreateOpen, setIsCreateOpen] = useState(false);
  const [isEditOpen, setIsEditOpen] = useState(false);
  const [serverToDelete, setServerToDelete] = useState<Server | null>(null);
  const [editingSubserver, setEditingSubserver] = useState<Server | null>(null);
  const [submitting, setSubmitting] = useState(false);
  const [syncingSubusers, setSyncingSubusers] = useState<string | null>(null);

  // Form states for Create
  const [createName, setCreateName] = useState('');
  const [createDescription, setCreateDescription] = useState('');
  const [createEggUuid, setCreateEggUuid] = useState<string | null>(null);
  const [createCpu, setCreateCpu] = useState<number>(100);
  const [createMemory, setCreateMemory] = useState<number>(1024);
  const [createDisk, setCreateDisk] = useState<number>(2048);
  const [createAllocations, setCreateAllocations] = useState<number>(1);
  const [createDatabases, setCreateDatabases] = useState<number>(0);
  const [createBackups, setCreateBackups] = useState<number>(0);
  const [createSchedules, setCreateSchedules] = useState<number>(0);
  const [createSyncSubusers, setCreateSyncSubusers] = useState(true);

  // Form states for Edit
  const [editName, setEditName] = useState('');
  const [editDescription, setEditDescription] = useState('');
  const [editCpu, setEditCpu] = useState<number>(100);
  const [editMemory, setEditMemory] = useState<number>(1024);
  const [editDisk, setEditDisk] = useState<number>(2048);
  const [editAllocations, setEditAllocations] = useState<number>(1);
  const [editDatabases, setEditDatabases] = useState<number>(0);
  const [editBackups, setEditBackups] = useState<number>(0);
  const [editSchedules, setEditSchedules] = useState<number>(0);

  const loadSplitterData = async () => {
    if (!currentServer?.uuid) return;
    try {
      setLoading(true);
      const res = await getClientSplitter(currentServer.uuid);
      setData(res);
    } catch (err) {
      addToast(httpErrorToHuman(err), 'error');
    } finally {
      setLoading(false);
    }
  };

  const loadNests = async () => {
    if (!currentServer?.uuid) return;
    try {
      const nests = await getClientSplitterNests(currentServer.uuid);
      setNestsData(nests);
      // Pick first egg by default if available
      const firstNest = Object.values(nests)[0];
      if (firstNest && firstNest[0]) {
        setCreateEggUuid(firstNest[0].uuid);
      }
    } catch {
      // Ignored
    }
  };

  useEffect(() => {
    loadSplitterData();
    loadNests();
  }, [currentServer?.uuid]);

  // Available eggs list for select dropdown
  const eggSelectOptions = useMemo(() => {
    const options: { group: string; items: { value: string; label: string }[] }[] = [];
    for (const [nestName, eggs] of Object.entries(nestsData)) {
      options.push({
        group: nestName,
        items: eggs.map((e) => ({
          value: e.uuid,
          label: e.name,
        })),
      });
    }
    return options;
  }, [nestsData]);

  const handleOpenCreate = () => {
    if (!data) return;
    const maxCpu = Math.max(1, data.resources.remaining_display.cpu);
    const maxMem = Math.max(256, data.resources.remaining_display.memory);
    const maxDisk = Math.max(512, data.resources.remaining_display.disk);

    setCreateName('');
    setCreateDescription('');
    setCreateCpu(Math.min(100, maxCpu));
    setCreateMemory(Math.min(1024, maxMem));
    setCreateDisk(Math.min(2048, maxDisk));
    setCreateAllocations(1);
    setCreateDatabases(0);
    setCreateBackups(0);
    setCreateSchedules(0);
    setCreateSyncSubusers(true);
    setIsCreateOpen(true);
  };

  const handleCreateSubmit = async () => {
    if (!currentServer?.uuid || !createEggUuid) return;
    if (!createName.trim()) {
      addToast('Server name is required', 'error');
      return;
    }

    try {
      setSubmitting(true);
      await createSplit(currentServer.uuid, {
        name: createName.trim(),
        description: createDescription.trim() || undefined,
        egg_uuid: createEggUuid,
        cpu: Number(createCpu),
        memory: Number(createMemory),
        disk: Number(createDisk),
        feature_limits: {
          allocations: Number(createAllocations),
          databases: Number(createDatabases),
          backups: Number(createBackups),
          schedules: Number(createSchedules),
        },
        sync_subusers: createSyncSubusers,
      });

      addToast('Child server created successfully', 'success');
      setIsCreateOpen(false);
      await loadSplitterData();
    } catch (err) {
      addToast(httpErrorToHuman(err), 'error');
    } finally {
      setSubmitting(false);
    }
  };

  const handleOpenEdit = (subserver: Server) => {
    setEditingSubserver(subserver);
    setEditName(subserver.name);
    setEditDescription(subserver.description || '');
    setEditCpu(subserver.limits.cpu);
    setEditMemory(subserver.limits.memory);
    setEditDisk(subserver.limits.disk);
    setEditAllocations(subserver.featureLimits.allocations);
    setEditDatabases(subserver.featureLimits.databases);
    setEditBackups(subserver.featureLimits.backups);
    setEditSchedules(subserver.featureLimits.schedules);
    setIsEditOpen(true);
  };

  const handleEditSubmit = async () => {
    if (!currentServer?.uuid || !editingSubserver) return;
    try {
      setSubmitting(true);
      await updateSplit(currentServer.uuid, editingSubserver.uuid, {
        name: editName.trim() || undefined,
        description: editDescription.trim() || undefined,
        cpu: Number(editCpu),
        memory: Number(editMemory),
        disk: Number(editDisk),
        feature_limits: {
          allocations: Number(editAllocations),
          databases: Number(editDatabases),
          backups: Number(editBackups),
          schedules: Number(editSchedules),
        },
      });

      addToast('Server limits updated successfully', 'success');
      setIsEditOpen(false);
      setEditingSubserver(null);
      await loadSplitterData();
    } catch (err) {
      addToast(httpErrorToHuman(err), 'error');
    } finally {
      setSubmitting(false);
    }
  };

  const handleDeleteConfirmed = async () => {
    if (!currentServer?.uuid || !serverToDelete) return;
    try {
      await deleteSplit(currentServer.uuid, serverToDelete.uuid);
      addToast('Child server deleted successfully', 'success');
      setServerToDelete(null);
      await loadSplitterData();
    } catch (err) {
      addToast(httpErrorToHuman(err), 'error');
    }
  };

  const handleSyncSubusers = async (subserver: Server) => {
    if (!currentServer?.uuid) return;
    try {
      setSyncingSubusers(subserver.uuid);
      await syncSubusers(currentServer.uuid, subserver.uuid);
      addToast(`Subusers synced to ${subserver.name}`, 'success');
    } catch (err) {
      addToast(httpErrorToHuman(err), 'error');
    } finally {
      setSyncingSubusers(null);
    }
  };

  if (loading && !data) {
    return (
      <ServerContentContainer title='Server Splitter'>
        <div className='flex justify-center items-center py-24'>
          <Spinner />
        </div>
      </ServerContentContainer>
    );
  }

  const remaining = data?.resources.remaining_display;
  const total = data?.resources.total;
  const subservers = data?.subservers ?? data?.servers ?? [];
  const parentServer = data?.parent ?? (currentServer?.uuid !== data?.master?.uuid ? data?.master : null);
  const maxSplits = total?.feature_limits.splits ?? 0;
  const canCreateMore = maxSplits > 0 && subservers.length < maxSplits;

  return (
    <ServerContentContainer
      title='Server Splitter'
      subtitle={
        maxSplits > 0 ? `Splits: ${subservers.length} of ${maxSplits} used` : 'Splits: Disabled for this server'
      }
      contentRight={
        canCreateMore ? (
          <Button onClick={handleOpenCreate} color='blue' leftSection={<FontAwesomeIcon icon={faPlus} />}>
            Create Split
          </Button>
        ) : undefined
      }
    >
      <Stack gap='lg'>
        {/* Child Server Notice */}
        {parentServer && (
          <Alert icon={<FontAwesomeIcon icon={faInfoCircle} />} title='Child Server' color='blue' radius='md'>
            <Group justify='space-between' align='center'>
              <Text size='sm'>
                This server is a child instance split from master server{' '}
                <Text span fw={700}>
                  {parentServer.name}
                </Text>
                . Its resources belong to the master server resource pool.
              </Text>
              <Link to={`/server/${parentServer.uuid}/splitter`}>
                <Button size='xs' variant='light' rightSection={<FontAwesomeIcon icon={faArrowRight} />}>
                  Go to Master Server
                </Button>
              </Link>
            </Group>
          </Alert>
        )}

        {/* Max splits reached or disabled notice */}
        {maxSplits === 0 && !parentServer && (
          <Alert
            icon={<FontAwesomeIcon icon={faExclamationTriangle} />}
            title='Splitting Disabled'
            color='yellow'
            radius='md'
          >
            Server splitting is not enabled for this server. Contact an administrator to allocate splits to your server.
          </Alert>
        )}

        {maxSplits > 0 && subservers.length >= maxSplits && (
          <Alert
            icon={<FontAwesomeIcon icon={faInfoCircle} />}
            title='Maximum Splits Reached'
            color='indigo'
            radius='md'
          >
            You have reached the maximum allowed child servers ({maxSplits}). Delete an existing child server to reclaim
            split capacity.
          </Alert>
        )}

        {/* Resource Pool Stat Cards */}
        {remaining && total && (
          <SimpleGrid cols={{ base: 1, sm: 2, md: 4 }} spacing='md'>
            {/* CPU */}
            <Card padding='md' radius='md' withBorder>
              <Group justify='space-between' mb='xs'>
                <Text size='xs' c='dimmed' fw={700} tt='uppercase'>
                  CPU Capacity
                </Text>
                <FontAwesomeIcon icon={faMicrochip} className='text-blue-400' />
              </Group>
              <Title order={3} fw={700} mb='xs'>
                {remaining.cpu}%
                <Text span size='sm' c='dimmed' fw={500}>
                  {' '}
                  / {total.cpu}% available
                </Text>
              </Title>
              <Progress
                value={total.cpu > 0 ? ((total.cpu - remaining.cpu) / total.cpu) * 100 : 0}
                color='blue'
                size='sm'
                radius='xl'
              />
            </Card>

            {/* Memory */}
            <Card padding='md' radius='md' withBorder>
              <Group justify='space-between' mb='xs'>
                <Text size='xs' c='dimmed' fw={700} tt='uppercase'>
                  RAM Capacity
                </Text>
                <FontAwesomeIcon icon={faMemory} className='text-emerald-400' />
              </Group>
              <Title order={3} fw={700} mb='xs'>
                {formatBytes(remaining.memory)}
                <Text span size='sm' c='dimmed' fw={500}>
                  {' '}
                  / {formatBytes(total.memory)} available
                </Text>
              </Title>
              <Progress
                value={total.memory > 0 ? ((total.memory - remaining.memory) / total.memory) * 100 : 0}
                color='teal'
                size='sm'
                radius='xl'
              />
            </Card>

            {/* Disk */}
            <Card padding='md' radius='md' withBorder>
              <Group justify='space-between' mb='xs'>
                <Text size='xs' c='dimmed' fw={700} tt='uppercase'>
                  Disk Space
                </Text>
                <FontAwesomeIcon icon={faHdd} className='text-purple-400' />
              </Group>
              <Title order={3} fw={700} mb='xs'>
                {formatBytes(remaining.disk)}
                <Text span size='sm' c='dimmed' fw={500}>
                  {' '}
                  / {formatBytes(total.disk)} available
                </Text>
              </Title>
              <Progress
                value={total.disk > 0 ? ((total.disk - remaining.disk) / total.disk) * 100 : 0}
                color='grape'
                size='sm'
                radius='xl'
              />
            </Card>

            {/* Splits count */}
            <Card padding='md' radius='md' withBorder>
              <Group justify='space-between' mb='xs'>
                <Text size='xs' c='dimmed' fw={700} tt='uppercase'>
                  Child Instances
                </Text>
                <FontAwesomeIcon icon={faNetworkWired} className='text-amber-400' />
              </Group>
              <Title order={3} fw={700} mb='xs'>
                {subservers.length}
                <Text span size='sm' c='dimmed' fw={500}>
                  {' '}
                  / {maxSplits} slots
                </Text>
              </Title>
              <Progress
                value={maxSplits > 0 ? (subservers.length / maxSplits) * 100 : 0}
                color='orange'
                size='sm'
                radius='xl'
              />
            </Card>
          </SimpleGrid>
        )}

        {/* Child Servers List */}
        <div>
          <Title order={4} mb='md' fw={600}>
            Child Servers ({subservers.length})
          </Title>

          {subservers.length === 0 ? (
            <Card padding='xl' radius='md' withBorder className='text-center py-12'>
              <Stack align='center' gap='md'>
                <div className='w-16 h-16 rounded-full bg-blue-500/10 flex items-center justify-center text-blue-400 text-2xl mb-1'>
                  <FontAwesomeIcon icon={faServer} />
                </div>
                <div>
                  <Text fw={600} size='lg'>
                    No child servers created yet
                  </Text>
                  <Text size='sm' c='dimmed' maw={450} mx='auto' mt='xs'>
                    Carve out dedicated resources from this server to launch separate child instances with their own
                    console, files, and eggs.
                  </Text>
                </div>
                {canCreateMore && (
                  <Button onClick={handleOpenCreate} color='blue' leftSection={<FontAwesomeIcon icon={faPlus} />}>
                    Create Split
                  </Button>
                )}
              </Stack>
            </Card>
          ) : (
            <SimpleGrid cols={{ base: 1, md: 2 }} spacing='md'>
              {subservers.map((sub) => {
                const allocationStr = sub.allocation ? `${sub.allocation.ip}:${sub.allocation.port}` : 'No allocation';

                return (
                  <Card key={sub.uuid} padding='lg' radius='md' withBorder shadow='sm'>
                    <Stack gap='md'>
                      {/* Card Header */}
                      <Group justify='space-between' wrap='nowrap'>
                        <div>
                          <Group gap='xs'>
                            <Title order={4} fw={600} lineClamp={1}>
                              {sub.name}
                            </Title>
                            {sub.egg && (
                              <Badge size='xs' variant='light' color='cyan'>
                                {sub.egg.name}
                              </Badge>
                            )}
                          </Group>
                          <Text size='xs' c='dimmed' mt={2}>
                            {allocationStr}
                          </Text>
                        </div>
                        <Badge size='sm' variant='dot' color={sub.isSuspended ? 'red' : 'green'}>
                          {sub.isSuspended ? 'Suspended' : 'Active'}
                        </Badge>
                      </Group>

                      {sub.description && (
                        <Text size='xs' c='dimmed' lineClamp={2}>
                          {sub.description}
                        </Text>
                      )}

                      {/* Resource badges */}
                      <SimpleGrid cols={3} spacing='xs'>
                        <div className='bg-zinc-800/40 dark:bg-zinc-900/60 p-2.5 rounded-lg border border-zinc-700/30 text-center'>
                          <Text size='xs' c='dimmed'>
                            CPU
                          </Text>
                          <Text fw={700} size='sm'>
                            {sub.limits.cpu}%
                          </Text>
                        </div>
                        <div className='bg-zinc-800/40 dark:bg-zinc-900/60 p-2.5 rounded-lg border border-zinc-700/30 text-center'>
                          <Text size='xs' c='dimmed'>
                            Memory
                          </Text>
                          <Text fw={700} size='sm'>
                            {formatBytes(sub.limits.memory)}
                          </Text>
                        </div>
                        <div className='bg-zinc-800/40 dark:bg-zinc-900/60 p-2.5 rounded-lg border border-zinc-700/30 text-center'>
                          <Text size='xs' c='dimmed'>
                            Disk
                          </Text>
                          <Text fw={700} size='sm'>
                            {formatBytes(sub.limits.disk)}
                          </Text>
                        </div>
                      </SimpleGrid>

                      {/* Feature limits pill */}
                      <Group gap='xs'>
                        <Badge size='xs' variant='outline' color='gray'>
                          Ports: {sub.featureLimits.allocations}
                        </Badge>
                        <Badge size='xs' variant='outline' color='gray'>
                          DBs: {sub.featureLimits.databases}
                        </Badge>
                        <Badge size='xs' variant='outline' color='gray'>
                          Backups: {sub.featureLimits.backups}
                        </Badge>
                        <Badge size='xs' variant='outline' color='gray'>
                          Schedules: {sub.featureLimits.schedules}
                        </Badge>
                      </Group>

                      {/* Actions */}
                      <Group justify='space-between' mt='xs'>
                        <Link to={`/server/${sub.uuid}`}>
                          <Button size='xs' color='blue' rightSection={<FontAwesomeIcon icon={faArrowRight} />}>
                            Open Console
                          </Button>
                        </Link>

                        <Group gap='xs'>
                          <Tooltip label='Sync master server subusers & permissions to this child server'>
                            <Button
                              size='xs'
                              variant='light'
                              color='teal'
                              loading={syncingSubusers === sub.uuid}
                              onClick={() => handleSyncSubusers(sub)}
                              leftSection={<FontAwesomeIcon icon={faUsers} />}
                            >
                              Sync Users
                            </Button>
                          </Tooltip>

                          <Button
                            size='xs'
                            variant='light'
                            color='gray'
                            onClick={() => handleOpenEdit(sub)}
                            leftSection={<FontAwesomeIcon icon={faEdit} />}
                          >
                            Resize
                          </Button>

                          <Button size='xs' variant='subtle' color='red' onClick={() => setServerToDelete(sub)}>
                            <FontAwesomeIcon icon={faTrash} />
                          </Button>
                        </Group>
                      </Group>
                    </Stack>
                  </Card>
                );
              })}
            </SimpleGrid>
          )}
        </div>
      </Stack>

      {/* CREATE SPLIT MODAL */}
      <Modal opened={isCreateOpen} onClose={() => setIsCreateOpen(false)} title='Create Child Server' size='lg'>
        <Stack gap='md'>
          <TextInput
            label='Server Name'
            placeholder='e.g. Lobby Proxy'
            required
            value={createName}
            onChange={(e) => setCreateName(e.currentTarget.value)}
          />

          <TextInput
            label='Description'
            placeholder='Optional description'
            value={createDescription}
            onChange={(e) => setCreateDescription(e.currentTarget.value)}
          />

          {eggSelectOptions.length > 0 ? (
            <Select
              label='Server Template (Egg)'
              placeholder='Select Egg'
              required
              data={eggSelectOptions}
              value={createEggUuid}
              onChange={(val) => setCreateEggUuid(val ? String(val) : null)}
              searchable
            />
          ) : (
            <Alert color='yellow' icon={<FontAwesomeIcon icon={faExclamationTriangle} />}>
              No allowed eggs configured for splitting. Contact an administrator.
            </Alert>
          )}

          <Title order={5} mt='xs'>
            Resource Allocation
          </Title>

          <SimpleGrid cols={{ base: 1, sm: 3 }} spacing='md'>
            <NumberInput
              label='CPU Limit (%)'
              required
              min={1}
              max={remaining?.cpu ?? 100}
              value={createCpu}
              onChange={(val) => setCreateCpu(typeof val === 'number' ? val : 0)}
              description={`Available: ${remaining?.cpu ?? 0}%`}
            />

            <NumberInput
              label='Memory (MB)'
              required
              min={256}
              max={remaining?.memory ?? 1024}
              value={createMemory}
              onChange={(val) => setCreateMemory(typeof val === 'number' ? val : 0)}
              description={`Available: ${formatBytes(remaining?.memory ?? 0)}`}
            />

            <NumberInput
              label='Disk (MB)'
              required
              min={512}
              max={remaining?.disk ?? 2048}
              value={createDisk}
              onChange={(val) => setCreateDisk(typeof val === 'number' ? val : 0)}
              description={`Available: ${formatBytes(remaining?.disk ?? 0)}`}
            />
          </SimpleGrid>

          <Title order={5} mt='xs'>
            Feature Limits
          </Title>

          <SimpleGrid cols={{ base: 2, sm: 4 }} spacing='md'>
            <NumberInput
              label='Allocations'
              min={1}
              max={remaining?.feature_limits?.allocations ?? 1}
              value={createAllocations}
              onChange={(val) => setCreateAllocations(typeof val === 'number' ? val : 1)}
              description={remaining ? `Available: ${remaining.feature_limits.allocations}` : undefined}
            />
            <NumberInput
              label='Databases'
              min={0}
              value={createDatabases}
              onChange={(val) => setCreateDatabases(typeof val === 'number' ? val : 0)}
            />
            <NumberInput
              label='Backups'
              min={0}
              value={createBackups}
              onChange={(val) => setCreateBackups(typeof val === 'number' ? val : 0)}
            />
            <NumberInput
              label='Schedules'
              min={0}
              value={createSchedules}
              onChange={(val) => setCreateSchedules(typeof val === 'number' ? val : 0)}
            />
          </SimpleGrid>

          <Switch
            label='Sync subusers and permissions to child server'
            checked={createSyncSubusers}
            onChange={(e) => setCreateSyncSubusers(e.currentTarget.checked)}
            mt='xs'
          />

          <ModalFooter>
            <Button variant='default' onClick={() => setIsCreateOpen(false)}>
              Cancel
            </Button>
            <Button color='blue' loading={submitting} onClick={handleCreateSubmit} disabled={!createEggUuid}>
              Create Server
            </Button>
          </ModalFooter>
        </Stack>
      </Modal>

      {/* EDIT / RESIZE MODAL */}
      <Modal
        opened={isEditOpen}
        onClose={() => setIsEditOpen(false)}
        title={`Resize ${editingSubserver?.name ?? 'Child Server'}`}
        size='lg'
      >
        <Stack gap='md'>
          <TextInput label='Server Name' value={editName} onChange={(e) => setEditName(e.currentTarget.value)} />

          <TextInput
            label='Description'
            value={editDescription}
            onChange={(e) => setEditDescription(e.currentTarget.value)}
          />

          <Title order={5} mt='xs'>
            Resource Limits
          </Title>

          <SimpleGrid cols={{ base: 1, sm: 3 }} spacing='md'>
            <NumberInput
              label='CPU (%)'
              required
              min={1}
              max={editingSubserver && remaining ? editingSubserver.limits.cpu + remaining.cpu : 100}
              value={editCpu}
              onChange={(val) => setEditCpu(typeof val === 'number' ? val : 0)}
              description={
                editingSubserver && remaining ? `Max: ${editingSubserver.limits.cpu + remaining.cpu}%` : undefined
              }
            />

            <NumberInput
              label='Memory (MB)'
              required
              min={256}
              max={editingSubserver && remaining ? editingSubserver.limits.memory + remaining.memory : 1024}
              value={editMemory}
              onChange={(val) => setEditMemory(typeof val === 'number' ? val : 0)}
              description={
                editingSubserver && remaining
                  ? `Max: ${formatBytes(editingSubserver.limits.memory + remaining.memory)}`
                  : undefined
              }
            />

            <NumberInput
              label='Disk (MB)'
              required
              min={512}
              max={editingSubserver && remaining ? editingSubserver.limits.disk + remaining.disk : 2048}
              value={editDisk}
              onChange={(val) => setEditDisk(typeof val === 'number' ? val : 0)}
              description={
                editingSubserver && remaining
                  ? `Max: ${formatBytes(editingSubserver.limits.disk + remaining.disk)}`
                  : undefined
              }
            />
          </SimpleGrid>

          <Title order={5} mt='xs'>
            Feature Limits
          </Title>

          <SimpleGrid cols={{ base: 2, sm: 4 }} spacing='md'>
            <NumberInput
              label='Allocations'
              min={1}
              max={
                editingSubserver && remaining
                  ? editingSubserver.featureLimits.allocations + remaining.feature_limits.allocations
                  : 1
              }
              value={editAllocations}
              onChange={(val) => setEditAllocations(typeof val === 'number' ? val : 1)}
              description={
                editingSubserver && remaining
                  ? `Max: ${editingSubserver.featureLimits.allocations + remaining.feature_limits.allocations}`
                  : undefined
              }
            />
            <NumberInput
              label='Databases'
              min={0}
              value={editDatabases}
              onChange={(val) => setEditDatabases(typeof val === 'number' ? val : 0)}
            />
            <NumberInput
              label='Backups'
              min={0}
              value={editBackups}
              onChange={(val) => setEditBackups(typeof val === 'number' ? val : 0)}
            />
            <NumberInput
              label='Schedules'
              min={0}
              value={editSchedules}
              onChange={(val) => setEditSchedules(typeof val === 'number' ? val : 0)}
            />
          </SimpleGrid>

          <ModalFooter>
            <Button variant='default' onClick={() => setIsEditOpen(false)}>
              Cancel
            </Button>
            <Button color='blue' loading={submitting} onClick={handleEditSubmit}>
              Save Changes
            </Button>
          </ModalFooter>
        </Stack>
      </Modal>

      {/* DELETE CONFIRMATION MODAL */}
      <ConfirmationModal
        opened={!!serverToDelete}
        onClose={() => setServerToDelete(null)}
        title='Delete Child Server'
        confirm='Delete Server'
        confirmColor='red'
        onConfirmed={handleDeleteConfirmed}
      >
        <Text size='sm'>
          Are you sure you want to permanently delete child server{' '}
          <Text span fw={700}>
            {serverToDelete?.name}
          </Text>
          ?
        </Text>
        <Text size='sm' c='red.4' mt='xs'>
          This will delete all server files and databases permanently. Reclaimed resources will be returned to the
          master server pool.
        </Text>
      </ConfirmationModal>
    </ServerContentContainer>
  );
}
