# Chaos Tests

Tests for data integrity and availability under adverse conditions.

## kill-pod-during-write.sh
Sends bulk writes while killing the pod. Verifies no corruption after restart.
```bash
./tests/chaos/kill-pod-during-write.sh <endpoint> <write-key> <pod-name> <namespace>
```

## concurrent-load.sh
Concurrent writers and readers. Verifies no errors under load.
```bash
./tests/chaos/concurrent-load.sh <endpoint> <write-key> <writers> <readers> <iterations>
```
