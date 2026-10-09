import { useQuery } from "@tanstack/react-query";
import { settingsApi } from "@/lib/api";

export function useAvailableRelease() {
  return useQuery({
    queryKey: ["available-release"],
    queryFn: () => settingsApi.getAvailableReleaseVersion(),
    staleTime: 5 * 60 * 1000,
    retry: false,
  });
}
