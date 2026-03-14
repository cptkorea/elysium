# elysium

# Project Elysium: Log Based Storage Engine

Inspired by the [Log Structure Merge-Tree](https://github.com/keyvanakbary/learning-notes/blob/master/books/designing-data-intensive-applications.md#storage-and-retrieval) outlined in the Designing Data Intensive Applications book.

The project is broken down into the following components:
```
- common (WIP: shared data-structure library)
- ontos (WIP: log based storage engine library)
- logos (WIP: distributed consensus layer — Raft-based KV store backed by ontos)
- pneuma (WIP: scheduled task orchestrator with DAG-based workflows)
```

# Etymology
The word elysium was inspired by the [Trinity Processor](https://xenoblade.fandom.com/wiki/Trinity_Processor) in Xenoblade Chronicles 2 which holds three cores:
```
ontos: 'machine', with default mode of operation, no-opinion
logos: 'reason', the distributed consensus engine that coordinates the cluster
pneuma: 'breath/spirit', the heartbeat-driven task scheduler that brings workflows to life
```
<img src="docs/images/TrinityProcessor.webp" width=300)>