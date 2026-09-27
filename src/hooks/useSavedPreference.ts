import {
  useIsMutating,
  useMutation,
  useQuery,
  useQueryClient,
  type MutationFunction,
} from "@tanstack/react-query";
import { toast } from "sonner";

export function useSavedPreference<T, V = T>({
  queryKey,
  load,
  save,
  optimisticUpdate,
  errorMessage,
}: {
  queryKey: readonly string[];
  load: () => Promise<T>;
  save: MutationFunction<T, V>;
  optimisticUpdate: (previous: T | undefined, next: V) => T;
  errorMessage: string;
}) {
  const client = useQueryClient();
  const query = useQuery({
    queryKey,
    queryFn: load,
    staleTime: Infinity,
    retry: false,
  });
  const mutation = useMutation({
    mutationKey: queryKey,
    scope: { id: JSON.stringify(queryKey) },
    mutationFn: save,
    onMutate: async (next: V) => {
      await client.cancelQueries({ queryKey });
      client.setQueryData<T>(queryKey, (previous) =>
        optimisticUpdate(previous, next),
      );
    },
    onSuccess: (saved) => {
      if (client.isMutating({ mutationKey: queryKey }) === 1)
        client.setQueryData(queryKey, saved);
    },
    onError: () => toast.error(errorMessage),
    onSettled: () => {
      // Read the committed value after the queue drains. A previous optimistic
      // value may belong to another failed save and is not a safe rollback.
      if (client.isMutating({ mutationKey: queryKey }) === 1)
        return client.invalidateQueries({ queryKey });
    },
  });
  const saving = useIsMutating({ mutationKey: queryKey }) > 0;
  return { query, mutation, saving };
}
