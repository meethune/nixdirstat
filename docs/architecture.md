# Architecture

## Module Dependency Graph

```mermaid
graph TD
    main["main.rs"]
    lib["lib.rs"]
    cli["cli"]
    pipeline["pipeline"]
    scanner["scanner"]
    storage["storage"]
    analyzer["analyzer"]
    ui["ui"]
    platform["platform"]
    types["types"]
    error["error"]

    main --> lib
    lib --> cli
    lib --> pipeline
    lib --> storage
    lib --> ui
    lib --> platform

    cli --> types

    pipeline --> scanner
    pipeline --> storage
    pipeline --> analyzer
    pipeline --> types

    scanner --> types
    storage --> types
    analyzer --> storage
    analyzer --> types
    ui --> storage
    ui --> pipeline
    ui --> types
    platform --> types

    scanner -.-> error
    storage -.-> error
    pipeline -.-> error
    ui -.-> error

    style main fill:#4a9,stroke:#333,color:#fff
    style ui fill:#48c,stroke:#333,color:#fff
    style pipeline fill:#c64,stroke:#333,color:#fff
    style storage fill:#a84,stroke:#333,color:#fff
    style scanner fill:#a84,stroke:#333,color:#fff
```

## Scan Pipeline (Data Flow)

```mermaid
sequenceDiagram
    participant CLI as cli::parse()
    participant Lib as lib::run()
    participant Pipeline as pipeline
    participant Scanner as WalkdirScanner
    participant Writer as SqliteStorage
    participant Analyzer as analyzer
    participant UI as TUI

    CLI->>Lib: Command::Scan { path }
    Lib->>Pipeline: run_pipeline(config, cancel)

    par Stage 1+2: Concurrent scan and write
        Pipeline->>Scanner: spawn_blocking: scan()
        Scanner-->>Pipeline: EntryBatch via mpsc
        Pipeline->>Writer: spawn_blocking: insert_batch()
        Scanner-->>UI: ScanProgress via mpsc
    end

    Pipeline->>Analyzer: spawn_blocking: aggregate_directory_sizes()
    Analyzer->>Writer: update_directory_sizes()
    Pipeline->>Writer: save_scan_metadata()
    Pipeline-->>Lib: PipelineResult

    alt Interactive mode
        Lib->>UI: run_scan_ui() / run_explore_ui()
        UI->>Writer: query_entries(), query_directory_children()
    else Batch mode
        Lib->>Writer: finalize_for_export()
    end
```

## Storage Schema (v2)

```mermaid
erDiagram
    entries {
        INTEGER id PK
        BLOB path_bytes "lossless canonical path"
        TEXT path_text "lossy queryable path"
        BLOB parent_bytes
        TEXT parent_text
        INTEGER size "logical bytes"
        INTEGER allocated "physical bytes"
        INTEGER file_type "FileType discriminant"
        INTEGER mode "Unix permission bits"
        INTEGER uid
        INTEGER gid
        INTEGER mtime "Unix seconds"
        INTEGER inode
        INTEGER device
        INTEGER nlink
        INTEGER category "FileCategory discriminant"
    }

    scan_metadata {
        INTEGER id PK "always 1"
        TEXT root_path
        INTEGER started_at "Unix seconds"
        INTEGER duration_ms
        INTEGER file_count
        INTEGER total_size
        INTEGER schema_version
    }
```

## TUI Component Tree

```mermaid
graph TD
    App["AppState"]
    Scan["ScanProgressState"]
    Explore["ExplorerState"]
    Progress["progress view"]
    Explorer["explorer view"]
    DirTree["dir_tree widget"]
    Treemap["treemap widget"]
    Legend["extension_legend widget"]

    App --> Scan
    App --> Explore
    Scan --> Progress
    Explore --> Explorer
    Explorer --> DirTree
    Explorer --> Treemap
    Explorer --> Legend

    style App fill:#48c,stroke:#333,color:#fff
    style Explorer fill:#48c,stroke:#333,color:#fff
```

## Module Responsibilities

| Module | Purpose |
|--------|---------|
| `cli` | Argument parsing, bare-path resolution |
| `scanner` | `Scanner` trait, `WalkdirScanner` filesystem walker |
| `storage` | `ReadStorage`/`WriteStorage` traits, `SqliteStorage` impl |
| `pipeline` | Async coordinator: scanner → writer → aggregation |
| `analyzer` | Directory size aggregation, free space query |
| `ui` | Terminal setup/teardown, event loop, TUI views and widgets |
| `platform` | Platform-specific filesystem detection (Linux/macOS/FreeBSD) |
| `types` | Domain types: `FileEntry`, `FileCategory`, `ScanConfig`, etc. |
| `error` | `ScanError`, `StorageError`, `PipelineError`, `UiError` |
