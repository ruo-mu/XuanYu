<p align="center">
  <img src="../apps/desktop-ui/public/assets/xuanyu.png" alt="XuanYu Agent" width="180" />
</p>

<h1 align="center">XuanYu Agent · 玄语</h1>

<p align="center">
  <strong>Espacio de trabajo de agentes de IA de escritorio con enfoque local-first</strong><br/>
  <sub>El razonamiento y la orquestación se ejecutan en un motor Rust independiente; la memoria y los datos permanecen en tu propia máquina</sub>
</p>

<p align="center">
  <a href="../README.md"><img alt="简体中文" src="https://img.shields.io/badge/简体中文-d9d9d9"></a>
  <a href="./README.en.md"><img alt="English" src="https://img.shields.io/badge/English-d9d9d9"></a>
  <a href="./README.ja.md"><img alt="日本語" src="https://img.shields.io/badge/日本語-d9d9d9"></a>
  <a href="./README.es.md"><img alt="Español" src="https://img.shields.io/badge/Español-d9d9d9"></a>
  <a href="./README.fr.md"><img alt="Français" src="https://img.shields.io/badge/Français-d9d9d9"></a>
</p>

<p align="center">
  <a href="https://img.shields.io/badge/platform-Windows%2010%2B-blue"><img src="https://img.shields.io/badge/platform-Windows%2010%2B-blue" alt="Platform"/></a>
  <a href="https://img.shields.io/badge/version-0.1.0--preview-0ea5e9"><img src="https://img.shields.io/badge/version-0.1.0--preview-0ea5e9" alt="Version"/></a>
  <a href="https://img.shields.io/badge/tech-Electron%20%2B%20Rust%20%2B%20PostgreSQL-8b5cf6"><img src="https://img.shields.io/badge/tech-Electron%20%2B%20Rust%20%2B%20PostgreSQL-8b5cf6" alt="Tech Stack"/></a>
  <a href="https://img.shields.io/badge/engine-15%20crates-16a34a"><img src="https://img.shields.io/badge/engine-15%20crates-16a34a" alt="Cargo Workspace"/></a>
  <a href="./LICENSE"><img src="https://img.shields.io/badge/license-MIT-green" alt="License"/></a>
</p>

---

**XuanYu Agent (玄语)** es un espacio de trabajo de agentes de IA de escritorio con enfoque **local-first**. El razonamiento central, la ejecución de herramientas y la gobernanza de seguridad se ejecutan en un proceso de motor Rust nativo independiente; Electron solo se encarga de la interfaz y la interacción con el dispositivo. Las sesiones, la memoria a largo plazo y los registros de ejecución residen íntegramente en **tu propio PostgreSQL** (una base de datos portátil integrada o una ya existente): funciona sin conexión, los datos no salen de tu equipo y, además, puedes conectar cualquier modelo principal y capacidades de terceros.

> [!NOTE]
> Actualmente es una versión preliminar `0.1.0-preview`, con plataforma de publicación **Windows 10+**. macOS / Linux están en la [hoja de ruta](#hoja-de-ruta).

## Índice

- [Características](#características)
- [Arquitectura general](#arquitectura-general)
- [Estructura del workspace](#estructura-del-workspace)
- [Principios clave](#principios-clave)
- [Inicio rápido](#inicio-rápido)
- [Compilar el paquete de distribución](#compilar-el-paquete-de-distribución)
- [Requisitos](#requisitos)
- [Configuración](#configuración)
- [Modelo de seguridad](#modelo-de-seguridad)
- [Hoja de ruta](#hoja-de-ruta)
- [Comunidad y contribución](#comunidad-y-contribución)
- [Licencia](#licencia)

## Características

- **Motor nativo local** — El razonamiento y la orquestación se ejecutan en un proceso independiente `xuanyu-engine-server`, totalmente desacoplados de la UI. Si el motor falla, el shell puede detectarlo y reiniciarlo automáticamente; también se puede reutilizar sin interfaz (CLI / servicio headless).
- **Memoria a largo plazo** — Búsqueda semántica con PostgreSQL + pgvector como eje principal, combinada con búsqueda de texto completo e índice Tantivy local para una recuperación multi-ruta, lo que permite memoria entre sesiones y consolidación de hechos clave.
- **Mercado de complementos / MCP / skills** — Explora e instala complementos, servicios MCP y skills desde la propia aplicación para ampliar las capacidades del agente según sea necesario, sin tocar el código.
- **Integración multi-modelo** — Un asistente de configuración guía la conexión con los principales proveedores y permite enrutar cada tarea (títulos / planificación / implementación / revisión, etc.) a distintos modelos, equilibrando coste y calidad.
- **Colaboración multi-agente** — Descomposición de tareas basada en subagentes, combinada con un flujo de trabajo de «revisión - fusión» para completar tareas de ingeniería complejas.
- **Integración multicanal** — WeChat, Feishu, DingTalk, Telegram, Discord, QQ, Line, WhatsApp y otras IM externas se conectan a un mismo conjunto de conversaciones y memoria.
- **Gobernanza de seguridad** — Lista negra de comandos, clasificación de instrucciones git destructivas, validación de ámbitos de rutas y sandbox del SO; las operaciones de alto riesgo requieren siempre aprobación humana.
- **Reenvío de edición reversible** — Al editar y reenviar cualquier mensaje de usuario del historial, solo se revierten con precisión los cambios de archivos que la IA produjo en ese turno (instantáneas de contenido por archivo + reversión diferencial), conservando tu trabajo no relacionado y sin confirmar.
- **Detalles de rendimiento** — La ejecución especulativa oculta la latencia de ida y vuelta de las herramientas, la compresión automática de contexto y la continuación al truncar evitan interrupciones en tareas largas.
- **Experiencia de escritorio moderna** — Electron + antd, temas claro / oscuro, Markdown en streaming, resaltado de código y vista de Diff integrados.

## Arquitectura general

```text
Capa de renderizado (React / Vite / antd)
        │  IPC / JSON-RPC
        ▼
Motor Rust (xuanyu-engine-server)
        │  SQL
        ▼
PostgreSQL + pgvector
```

Tres líneas de diseño principales:

- **Desacoplamiento de procesos** — El motor es un proceso independiente que se comunica en ambos sentidos con el shell de escritorio mediante JSON-RPC sobre stdin/stdout (ver [engine_server.rs](../crates/xuanyu-cli/src/bin/engine_server.rs)). No depende de la interfaz, por lo que puede ejecutarse y probarse por separado.
- **Impulsado por eventos** — Todos los cambios asíncronos del motor (incrementos de turno, progreso de herramientas, solicitudes de permisos, actualizaciones de memoria, etc.) pasan por un `eventbus` unificado y se serializan como `AppEvent` para enviarse activamente a la UI. La UI no sondea; admite de forma natural suscripciones multidispositivo.
- **Almacenamiento como única fuente de verdad** — Sesiones, mensajes, memoria, registros de ejecución y logs de eventos se guardan íntegramente en PostgreSQL, y tanto la base portátil integrada como una existente usan el mismo esquema.

## Estructura del workspace

```text
apps/
  desktop/            Electron shell (proceso principal / preload / configuración de empaquetado)
  desktop-ui/         capa de renderizado (React + Vite + antd)
crates/               15 crates de Rust (Cargo workspace)
proto/                definiciones de protocolo entre el motor y el shell (app.proto / session.proto)
scripts/              generación de código IPC, empaquetado y scripts de smoke
```

Resumen de los crates del lado del motor:

| Crate | Responsabilidad |
|-------|------|
| `xuanyu-core` | Capa de integración del motor: AppCore, sesiones / memoria / mercado / canales |
| `xuanyu-execution` | Runtime de ejecución: bucle principal / bucle de herramientas / ensamblado de contexto / presupuesto de tokens |
| `xuanyu-session` | Dominio de sesión: SessionManager / hilos / aislamiento / cola de mensajes |
| `xuanyu-memory` | Memoria a largo plazo: búsqueda semántica pgvector + FTS + Tantivy + embedding |
| `xuanyu-tools` | Runtime de herramientas: MCP / MCP remoto / sandbox del SO / límites de salida |
| `xuanyu-governance` | Gobernanza de seguridad: validación de comandos / clasificación git / validación de rutas / reversión / contabilidad |
| `xuanyu-channels` | Canales externos: WeChat / Feishu / DingTalk / Telegram / Discord, etc. |
| `xuanyu-eventbus` | Bus de eventos: spine / router / transformer / backpressure |
| `xuanyu-storage` | Almacenamiento y plano de control: pool de conexiones / tabla de sesiones / PostgreSQL embebido |
| `xuanyu-protocol` | Contrato de protocolo: Envelope / AppEvent / AppError / DTO |
| `xuanyu-cli` | CLI y engine-server (incluye el comando auxiliar `db-external`) |
| `xuanyu-desktop-host` | Puente del host de escritorio: expone comandos IPC con transcodificación automática |
| `xuanyu-agent-bridge` | Puente de agentes: vinculación del workspace de subagentes |
| `xuanyu-agents` | Runtime de subagentes: watchdog / assess_merge |
| `xuanyu-extensions` | Extensiones de capacidad: carga de complementos / MCP / skills |

## Principios clave

### Ciclo de vida de una sesión

Cuando un mensaje de usuario entra en [AppCore](../crates/xuanyu-core/src/lib.rs), pasa por cuatro pasos:

1. **Ensamblado de contexto** — [prepare_turn](../crates/xuanyu-execution/src/context_assembler.rs) decide, según `loadMode` (lazy / balanced / eager), qué contenido inyectar en este turno: historial, resultados de recuperación de memoria, hechos clave, etc.
2. **Bucle principal** — [run_main_turn_loop](../crates/xuanyu-execution/src/main_turn_loop.rs) llama al modelo repetidamente dentro del límite de iteraciones y gestiona las llamadas a herramientas, con soporte para una fecha límite de reloj de pared `deadline_ms` y un límite blando de iteraciones.
3. **Distribución de herramientas** — Detecta llamadas a herramientas nativas o «de impresión» (compatible con modelos que no admiten tool call de forma nativa) y las entrega a `xuanyu-tools` para su ejecución; el resultado se escribe de vuelta en la transcripción y pasa al siguiente turno.
4. **Cierre** — Cuando el modelo deja de solicitar herramientas, el turno finaliza con `final_text`.

Detalles de ingeniería clave:

- **Compresión de contexto** — Las constantes de presupuesto están en [session_context.rs](../crates/xuanyu-execution/src/session_context.rs) y se activan mediante la configuración `contextManagement.autoCompactThreshold`; recortan resultados de herramientas antiguos y añaden un handoff summary para evitar discontinuidades de información.
- **Continuación al truncar** — Cuando `finish_reason` es `length` / `max_tokens` no se considera una respuesta final: se inyecta un aviso de continuación y se pasa al siguiente turno.
- **Ejecución especulativa** — [speculative_tools.rs](../crates/xuanyu-core/src/speculative_tools.rs) pone en cola de ejecución los parámetros en cuanto se cierran durante la generación en streaming, y al hacer dispatch toma el resultado directamente, ocultando la latencia de ida y vuelta de las herramientas.

### Memoria a largo plazo

Implementada por [xuanyu-memory](../crates/xuanyu-memory/src), una pila de memoria multi-backend y con múltiples rutas de recuperación:

- `embedding` admite tres orígenes: API compatible con OpenAI, inferencia local y hash sin conexión (por defecto; funciona desde el primer momento incluso sin API Key).
- La búsqueda vectorial pgvector es la principal (distancia coseno Top-N); tras ≥3 fallos consecutivos se abre el circuito automáticamente y se degrada a búsqueda por palabras clave con PostgreSQL FTS.
- El índice Tantivy local ([tantivy_store.rs](../crates/xuanyu-memory/src/tantivy_store.rs)) sirve como recuperación de respaldo para el modo totalmente sin conexión.

### Comunicación frontend-backend

El contrato de protocolo está en [xuanyu-protocol](../crates/xuanyu-protocol/src/lib.rs): el sobre genérico `Envelope<T>` transporta `correlation_id` / `trace_id` para la correlación de solicitudes y el trazado de la cadena, y un `AppError` unificado (con error code / recoverable / suggestion) sustituye a los códigos de error sin procesar.

El puente IPC del frontend al motor se genera automáticamente mediante scripts, para evitar la deriva de contrato que provocaría escribirlo a mano:

| Script | Función |
|------|------|
| [gen-desktop-dispatch.py](../scripts/gen-desktop-dispatch.py) | Analiza las firmas de funciones de `xuanyu-desktop-host` y genera el envoltorio de dispatch |
| [gen-desktop-ipc-wrappers.py](../scripts/gen-desktop-ipc-wrappers.py) | Genera los envoltorios de llamada en TypeScript |
| [gen-ipc-command-map.py](../scripts/gen-ipc-command-map.py) | Verifica la cobertura de comandos en ambos lados |

Después de modificar las capacidades del motor, ejecuta `pnpm gen:desktop-ipc` para alinear el contrato entre frontend y backend.

## Inicio rápido

### Uso directo (recomendado)

Ve a [Releases](https://github.com/ruo-mu/XuanYu/releases) y descarga el último `XuanYu Agent Setup <versión>.exe`, luego haz doble clic para instalarlo. En el primer inicio se abre el asistente de configuración: elige un modelo y empieza a usarlo.

El instalador admite dos modos de base de datos:

| Modo | Descripción |
|------|------|
| **PostgreSQL portátil integrado** | Se descarga e inicializa automáticamente (incluye la extensión pgvector), **funciona desde el primer momento** y no requiere ninguna configuración adicional |
| **Conectar a una base de datos existente** | Introduce la cadena de conexión y prueba la conectividad (con un tiempo de espera integrado de 8 segundos) para reutilizar tu instancia de PostgreSQL 15+ existente |

### Desarrollo desde el código fuente

Requisitos previos: Rust stable (`x86_64-pc-windows-msvc`), Node.js 20+ y pnpm.

```bash
pnpm install
pnpm dev          # inicia Electron y el motor local
```

Otros comandos habituales:

```bash
pnpm typecheck    # comprobación de tipos del proceso principal + la capa de renderizado
pnpm smoke        # prueba de humo
pnpm gen:desktop-ipc   # regenerar el contrato IPC tras cambios en las capacidades del motor
```

## Compilar el paquete de distribución

```bash
pnpm dist
```

Este comando realiza, en orden: compilar el motor en modo release → construir el frontend → empaquetar el instalador NSIS. El resultado queda en `apps/desktop/release-dist/`.

> [!IMPORTANT]
> Volver a ejecutar `pnpm install` sobrescribe el parche de la plantilla NSIS de electron-builder. El repositorio ya incluye un hook `postinstall` que lo vuelve a aplicar automáticamente; si el instalador falla, comprueba primero que el parche esté activo.

## Requisitos

| Categoría | Requisito |
|------|------|
| Sistema operativo | Windows 10 o superior (plataforma actual de publicación) |
| Runtime | PostgreSQL 15+ (solo necesario en «Conectar a una base de datos existente»; el modo portátil se instala automáticamente) |
| Desarrollo | Node.js 20+, pnpm 10+, Rust stable + target MSVC |

## Configuración

La configuración de ejecución y las credenciales se guardan en `C:\Users\<usuario>\.xuanyu\xuanyu.json`.

Se puede acceder al asistente de configuración por dos vías: el **menú «Archivo»** de la barra de título o el **botón «Asistente de configuración»** de la página de ajustes del sistema; tras la primera configuración, puedes volver a entrar cuando quieras. Al escribir la configuración, la cadena de conexión existente se **fusiona en lugar de sobrescribirse**, para no perder la conexión a la base de datos en una segunda configuración. La lógica de validación de la configuración está en [config_schema.rs](../crates/xuanyu-core/src/config_schema.rs).

## Modelo de seguridad

- **Lista negra de comandos** — Bloquea comandos peligrosos como `rm -rf /`, `sudo rm`, `chmod 777`, `eval` ([command_validator.rs](../crates/xuanyu-governance/src/command_validator.rs)).
- **Clasificación git** — Las instrucciones destructivas (`rebase` / `reset` / `clean` / `filter-*`, etc.) se rechazan siempre ([git_command_policy.rs](../crates/xuanyu-governance/src/git_command_policy.rs)).
- **Validación de rutas** — Las operaciones de archivos se limitan al ámbito aislado de la sesión ([path_validator.rs](../crates/xuanyu-governance/src/path_validator.rs)).
- **Sandbox del SO** — En macOS `sandbox-exec` y en Linux `bwrap` envuelven los comandos ([os_sandbox.rs](../crates/xuanyu-tools/src/os_sandbox.rs)).
- **Aprobación humana** — Las llamadas a herramientas de alto riesgo activan `PermissionRequested`; tras la aprobación en la ventana emergente de la UI, regresan mediante `PermissionAnswered`, con contabilidad completa ([turn_ledger.rs](../crates/xuanyu-governance/src/turn_ledger.rs)).
- **Seguridad de reversión** — El retroceso del workspace usa restauración diferencial por instantáneas de contenido y **nunca** emplea comandos destructivos como `git reset --hard`; los archivos que el usuario haya modificado manualmente se omiten en la restauración y se añaden a la lista de conflictos.

Para saber cómo reportar problemas de seguridad, consulta [SECURITY.md](../.github/SECURITY.md).

## Hoja de ruta

- [ ] Soporte multiplataforma (macOS / Linux)
- [ ] Sesiones en tiempo real y suscripción impulsada por eventos
- [ ] Distribución en línea y actualización automática del mercado de agentes / skills
- [ ] Plantillas más ricas de revisión y políticas de permisos
- [ ] Optimización del arranque en frío y del rendimiento
- [ ] Visualización de la memoria y reordenación de la recuperación híbrida

## Comunidad y contribución

- [CONTRIBUTING.md](../.github/CONTRIBUTING.md) — Normas y flujo para participar en el desarrollo
- [CODE_OF_CONDUCT.md](../.github/CODE_OF_CONDUCT.md) — Código de conducta de la comunidad
- [SECURITY.md](../.github/SECURITY.md) — Cómo reportar problemas de seguridad
- [Plantilla de Issue](../.github/ISSUE_TEMPLATE) — Reportes de bugs y sugerencias de funcionalidades

Si encuentras un bug o tienes una sugerencia de mejora, envíanos un Issue o un Pull Request.

## Licencia

Este proyecto se distribuye bajo la [MIT License](../LICENSE).