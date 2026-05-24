import { Table, Thead, Tr, Th, Td } from "@/components/ui/table";

export function ErrorsByCodeList({ data }: { data: Record<string, number> }) {
  const entries = Object.entries(data).sort((a, b) => b[1] - a[1]);
  if (entries.length === 0) {
    return <div className="text-muted-foreground">No errors recorded.</div>;
  }
  return (
    <Table>
      <Thead>
        <Tr><Th>Code</Th><Th className="text-right">Count</Th></Tr>
      </Thead>
      <tbody>
        {entries.map(([code, count]) => (
          <Tr key={code}>
            <Td className="font-mono">{code}</Td>
            <Td className="text-right tabular-nums">{count}</Td>
          </Tr>
        ))}
      </tbody>
    </Table>
  );
}
