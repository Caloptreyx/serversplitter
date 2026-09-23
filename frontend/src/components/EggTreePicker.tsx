import { faChevronDown, faChevronRight, faMagnifyingGlass } from '@fortawesome/free-solid-svg-icons';
import { FontAwesomeIcon } from '@fortawesome/react-fontawesome';
import { ActionIcon, Checkbox, Group, Input, Paper, ScrollArea, Stack, Text, TextInput } from '@mantine/core';
import { type ReactNode, useMemo, useState } from 'react';
import Button from '@/elements/buttons/Button.tsx';
import type { EggItem } from '../api/client.ts';

interface NestGroup {
  uuid: string;
  name: string;
  eggs: EggItem[];
}

interface EggTreePickerProps {
  label: string;
  description: string;
  eggs: EggItem[];
  value: string[];
  onChange: (value: string[]) => void;
  /** Extra controls shown next to the selection count. */
  actions?: ReactNode;
}

/** Nest -> egg checkbox tree: a nest checkbox toggles every egg shown under it. */
export default function EggTreePicker({ label, description, eggs, value, onChange, actions }: EggTreePickerProps) {
  const [search, setSearch] = useState('');
  // Nests that already hold a selection start expanded.
  const [expanded, setExpanded] = useState<Set<string>>(
    () => new Set(eggs.filter((egg) => value.includes(egg.uuid)).map((egg) => egg.nest_uuid)),
  );

  const selected = useMemo(() => new Set(value), [value]);

  const nests = useMemo(() => {
    const groups = new Map<string, NestGroup>();
    for (const egg of eggs) {
      let group = groups.get(egg.nest_uuid);
      if (!group) {
        group = { uuid: egg.nest_uuid, name: egg.nest_name, eggs: [] };
        groups.set(egg.nest_uuid, group);
      }
      group.eggs.push(egg);
    }
    return [...groups.values()];
  }, [eggs]);

  const query = search.trim().toLowerCase();
  const visibleNests = useMemo(() => {
    if (!query) return nests;
    return nests.flatMap((nest) => {
      if (nest.name.toLowerCase().includes(query)) return [nest];
      const matching = nest.eggs.filter((egg) => egg.name.toLowerCase().includes(query));
      return matching.length > 0 ? [{ ...nest, eggs: matching }] : [];
    });
  }, [nests, query]);

  const setEggs = (uuids: string[], checked: boolean) => {
    const next = new Set(selected);
    for (const uuid of uuids) {
      if (checked) {
        next.add(uuid);
      } else {
        next.delete(uuid);
      }
    }
    onChange([...next]);
  };

  const toggleExpanded = (uuid: string) =>
    setExpanded((prev) => {
      const next = new Set(prev);
      if (!next.delete(uuid)) next.add(uuid);
      return next;
    });

  return (
    <Input.Wrapper label={label} description={description} required>
      <Stack gap='xs' mt='xs'>
        <TextInput
          size='xs'
          placeholder='Search eggs or nests'
          leftSection={<FontAwesomeIcon icon={faMagnifyingGlass} />}
          value={search}
          onChange={(e) => setSearch(e.currentTarget.value)}
        />

        <Paper withBorder radius='sm' p='xs'>
          <ScrollArea.Autosize mah={280} type='auto'>
            {visibleNests.length === 0 ? (
              <Text size='sm' c='dimmed' ta='center' py='md'>
                {query ? 'No eggs match your search.' : 'No eggs on this panel.'}
              </Text>
            ) : (
              <Stack gap={4}>
                {visibleNests.map((nest) => {
                  const uuids = nest.eggs.map((egg) => egg.uuid);
                  const count = uuids.filter((uuid) => selected.has(uuid)).length;
                  // searching always shows the matching eggs
                  const open = query !== '' || expanded.has(nest.uuid);

                  return (
                    <div key={nest.uuid}>
                      <Group gap='xs' wrap='nowrap'>
                        <ActionIcon
                          variant='subtle'
                          color='gray'
                          size='sm'
                          disabled={query !== ''}
                          onClick={() => toggleExpanded(nest.uuid)}
                          aria-label={open ? `Collapse ${nest.name}` : `Expand ${nest.name}`}
                        >
                          <FontAwesomeIcon icon={open ? faChevronDown : faChevronRight} />
                        </ActionIcon>
                        <Checkbox
                          label={
                            <Text size='sm' fw={600}>
                              {nest.name}
                            </Text>
                          }
                          checked={count === uuids.length}
                          indeterminate={count > 0 && count < uuids.length}
                          onChange={() => setEggs(uuids, count < uuids.length)}
                        />
                        <Text size='xs' c='dimmed' ml='auto'>
                          {count}/{uuids.length}
                        </Text>
                      </Group>

                      {open && (
                        <Stack gap={6} pl={52} py={6}>
                          {nest.eggs.map((egg) => (
                            <Checkbox
                              key={egg.uuid}
                              label={egg.name}
                              checked={selected.has(egg.uuid)}
                              onChange={(e) => setEggs([egg.uuid], e.currentTarget.checked)}
                            />
                          ))}
                        </Stack>
                      )}
                    </div>
                  );
                })}
              </Stack>
            )}
          </ScrollArea.Autosize>
        </Paper>

        <Group justify='space-between'>
          <Text size='xs' c='dimmed'>
            {value.length} selected
          </Text>
          <Group gap='xs'>
            {actions}
            <Button size='xs' variant='subtle' color='gray' disabled={value.length === 0} onClick={() => onChange([])}>
              Clear
            </Button>
          </Group>
        </Group>
      </Stack>
    </Input.Wrapper>
  );
}
