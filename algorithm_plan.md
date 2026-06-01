Sí. Este sería un plan completo, pensado para implementar primero **Plan A: multi-cursor con territorios fijos**, pero dejando preparada la arquitectura para **Plan B: territorios dinámicos**.

## Objetivo técnico

Crear un scheduler generalizado por cursores donde:

```text
EST = MultiCursorScheduler con 1 cursor forward
LST = MultiCursorScheduler con 1 cursor backward
EST+LST = MultiCursorScheduler con 2 cursores: forward desde start + backward desde end
Start+middle = MultiCursorScheduler con 2 cursores forward y territorios fijos
```

El punto crítico es que EST/LST deben seguir comportándose exactamente igual que ahora. Actualmente EST tiene un único `ScheduleState` con `cursor`, `schedule`, `candidates` y `score`.  LST, por su parte, no es un algoritmo separado: espeja las ventanas, ejecuta EST y después deshace el espejo. 

---

# Milestone 0 — Baseline y tests de equivalencia actuales

**Objetivo:** congelar el comportamiento actual antes de tocar la arquitectura.

### Tareas

1. Crear tests de caracterización para EST.
2. Crear tests de caracterización para LST.
3. Añadir helper de comparación de schedules:

   * mismo número de placements;
   * mismos `task_id`;
   * mismos `start`;
   * mismos `end`.
4. Añadir fixtures pequeños y deterministas:

   * tareas sin soft constraints;
   * tareas con ventanas separadas;
   * tareas con ventanas solapadas;
   * caso con `endangered_threshold > 0`;
   * caso con `k_beams > 1`;
   * caso con `branching_factor > 1`.

### Acceptance criteria

```text
cargo test -p schedulers est
cargo test -p schedulers lst
```

deben pasar antes de cualquier refactor.

### Resultado esperado

Una suite que permita demostrar más adelante:

```text
EstScheduler actual == MultiCursorScheduler(single forward)
LstScheduler actual == MultiCursorScheduler(single backward)
```

---

# Milestone 1 — Introducir modelo de tipos sin cambiar comportamiento

**Objetivo:** crear las abstracciones del nuevo modelo, pero sin conectar todavía el algoritmo.

### Nuevos tipos recomendados

Crear módulo nuevo:

```text
schedulers/src/scheduler/cursor/
  mod.rs
  config.rs
  territory.rs
  frame.rs
  state.rs
  action.rs
```

Tipos base:

```rust
pub struct CursorId(pub usize);

pub enum CursorDirection {
    Forward,
    Backward,
}

pub enum CursorAnchor {
    HorizonStart,
    HorizonEnd,
    Fraction(f64),
    Mjd(Time<MJD>),
}

pub enum CursorTerritory {
    Fixed {
        start: Time<MJD>,
        end: Time<MJD>,
    },

    // No implementarlo todavía.
    // Solo reservar la forma conceptual para Plan B.
    Dynamic {
        // future
    },
}

pub struct CursorConfig {
    pub id: CursorId,
    pub anchor: CursorAnchor,
    pub direction: CursorDirection,
    pub territory: CursorTerritory,
}
```

Config global:

```rust
pub struct MultiCursorConfig {
    pub cursors: Vec<CursorConfig>,
    pub k_beams: usize,
    pub branching_factor: usize,
    pub endangered_threshold: u32,
    pub cursor_policy: CursorPolicy,
}

pub enum CursorPolicy {
    BestCandidateGlobal,
    RoundRobin,
}
```

Para la primera versión, implementaría solo:

```rust
CursorPolicy::BestCandidateGlobal
```

### Acceptance criteria

* El código compila.
* No cambia ningún test existente.
* EST/LST siguen usando el código actual.

---

# Milestone 2 — Separar “posición del cursor” de “territorio activo”

**Objetivo:** evitar que el Plan A quede hardcodeado y preparar Plan B.

Ahora EST refresca la cola con un periodo derivado directamente del cursor:

```rust
Period::new(state.cursor, horizon.end)
```

El beam search actual refresca candidatos contra ese periodo activo antes de expandir.  En multi-cursor eso debe pasar a ser:

```rust
let active_period = cursor.active_period(&state, horizon);
cursor.candidates.refresh(&active_period, endangered_threshold);
```

### API deseada

```rust
impl CursorRuntime {
    pub fn active_period(
        &self,
        state: &MultiCursorState,
        horizon: &Period<MJD>,
    ) -> Option<Period<MJD>> {
        match self.territory {
            CursorTerritory::Fixed { start, end } => {
                match self.direction {
                    CursorDirection::Forward => {
                        Period::new_checked(self.cursor, end)
                    }
                    CursorDirection::Backward => {
                        Period::new_checked(start, self.cursor)
                    }
                }
            }
            CursorTerritory::Dynamic { .. } => {
                todo!("Plan B")
            }
        }
    }
}
```

La idea importante:

```text
cursor.position != cursor.territory
```

Esto es lo que hará fácil evolucionar a Plan B.

### Acceptance criteria

* `CursorTerritory::Fixed` funciona.
* No hay midpoint hardcoded dentro del motor.
* El cálculo del periodo activo está centralizado en una función.

---

# Milestone 3 — Extender `CandidateQueue` para uso multi-cursor

**Objetivo:** reutilizar la cola EST sin duplicar lógica.

La cola actual encapsula candidatos, refresca EST/flexibilidad/endangerment y mantiene candidatos schedulables delante.  El candidato calcula `est`, `deadline` y `flexibility` a partir del horizonte activo. 

Para multi-cursor hacen falta pequeñas extensiones:

### Añadir métodos

```rust
impl CandidateQueue<'_> {
    pub fn candidate_task_id(&self, idx: usize) -> Option<TaskId>;

    pub fn schedulable_indices(&self) -> impl Iterator<Item = usize>;

    pub fn retain_unplaced(&mut self, schedule: &Schedule);

    pub fn clone_pop_at(&self, idx: usize) -> Option<Candidate>;
}
```

O mantener `pop_at`, pero el engine multi-cursor necesita poder construir acciones antes de mutar el estado.

### Problema a resolver

En multi-cursor, una misma tarea puede aparecer en la cola de varios cursores. Si el cursor A la coloca, el cursor B ya no debe poder colocarla.

Solución inicial simple:

```text
- al refrescar cada cursor, eliminar candidatos ya presentes en schedule;
- al aplicar una acción, volver a validar schedule.contains(task_id);
```

### Acceptance criteria

* EST actual no cambia.
* `CandidateQueue` sigue manteniendo el mismo orden.
* Se puede listar acciones sin mutar el estado.
* No se puede colocar dos veces el mismo `task_id`.

---

# Milestone 4 — Crear `MultiCursorState` y acciones `(cursor_id, candidate_idx)`

**Objetivo:** generalizar el beam search.

Estado nuevo:

```rust
pub struct MultiCursorState<'a> {
    pub schedule: Schedule,
    pub cursors: Vec<CursorRuntime<'a>>,
    pub score: f64,
}
```

Acción:

```rust
pub struct CursorAction {
    pub cursor_id: CursorId,
    pub candidate_idx: usize,
    pub rank: ActionRank,
}
```

El beam search actual expande un único beam probando candidatos de una única cola.  El nuevo flujo debe ser:

```text
for each live beam:
  refresh all cursors
  collect actions from all cursors
  sort actions
  try top branching_factor actions
  build child states
  score child states

globally keep top-k beams
```

### Política inicial de ranking

Para Plan A:

```text
BestCandidateGlobal:
  1. comparar por orden interno del candidato dentro de su cursor
  2. desempatar por cursor_id
```

Más adelante se puede hacer más sofisticado:

```text
- alternancia round-robin
- balance de carga entre cursores
- score preliminar por FOM
- prioridad por frontera más restringida
```

### Acceptance criteria

* El motor puede crear hijos desde múltiples cursores.
* Solo avanza el cursor que ha colocado la tarea.
* Los otros cursores permanecen en el estado.
* `k_beams` y `branching_factor` siguen teniendo semántica clara.

---

# Milestone 5 — Validación global de placements

**Objetivo:** garantizar que varios cursores no generen overlaps.

En EST de un cursor, el cursor lineal hace que los overlaps sean casi imposibles por construcción. En multi-cursor no. El `Schedule` ya tiene un índice de intervalos y permite consultar solapamientos.  

Añadir validación común:

```rust
fn validate_multi_cursor_placement(
    schedule: &Schedule,
    placement: &TaskPlacement,
    problem: &SchedulingProblem,
    territory: &Period<MJD>,
) -> Result<(), ScheduleError> {
    // 1. no duplicated task
    // 2. no overlap
    // 3. placement inside cursor territory
    // 4. block dependencies
}
```

Dependencias: EST ya valida que los predecessors estén colocados y terminen antes del candidato.  Esa lógica debe reutilizarse, no duplicarse.

### Acceptance criteria

* Dos cursores no pueden colocar tareas solapadas.
* Una tarea no puede colocarse dos veces.
* Una tarea no puede salir de su territorio fijo.
* Las dependencias de bloque siguen respetándose.

---

# Milestone 6 — Implementar single forward cursor y demostrar equivalencia con EST

**Objetivo:** primera versión funcional del nuevo motor.

Config:

```rust
MultiCursorConfig {
    cursors: vec![
        CursorConfig {
            id: CursorId(0),
            anchor: HorizonStart,
            direction: Forward,
            territory: Fixed {
                start: horizon.start,
                end: horizon.end,
            },
        }
    ],
    k_beams: est_config.k_beams,
    branching_factor: est_config.branching_factor,
    endangered_threshold: est_config.endangered_threshold,
    cursor_policy: BestCandidateGlobal,
}
```

### Tests

```text
est_current_equals_multicursor_single_forward_basic
est_current_equals_multicursor_single_forward_endangered
est_current_equals_multicursor_single_forward_beam
est_current_equals_multicursor_single_forward_soft_constraints
```

### Acceptance criteria

* Mismos placements exactos que EST.
* Mismo comportamiento con `k_beams`.
* Mismo comportamiento con `branching_factor`.
* Mismo comportamiento con `endangered_threshold`.

No cambiar todavía `EstScheduler` para delegar. Primero demostrar equivalencia.

---

# Milestone 7 — Implementar backward cursor y demostrar equivalencia con LST

**Objetivo:** hacer que `single backward` sea equivalente a LST actual.

Aquí hay que ir con cuidado. LST actual usa espejo temporal:

```text
original periods
  -> mirror_task_periods
  -> EstScheduler
  -> unmirror_schedule
```



Para preservar equivalencia, un cursor backward debería usar un `CursorFrame`:

```rust
pub enum CursorFrame {
    Identity,
    Mirrored { horizon: Period<MJD> },
}
```

Funciones:

```rust
fn to_local_time(...)
fn to_original_time(...)
fn to_local_period(...)
fn to_original_period(...)
fn to_local_periods(...)
fn placement_to_original(...)
```

Para un cursor backward:

```text
CandidateQueue trabaja en tiempo local espejado.
Schedule final se guarda en tiempo original.
```

### Tests

```text
lst_current_equals_multicursor_single_backward_basic
lst_current_equals_multicursor_single_backward_endangered
lst_current_equals_multicursor_single_backward_beam
lst_current_equals_multicursor_single_backward_soft_constraints
```

### Acceptance criteria

* `MultiCursorScheduler(single backward)` produce exactamente lo mismo que LST actual.
* No se rompe `LstScheduler`.
* La transformación backward se encapsula en `CursorFrame`.

---

# Milestone 8 — Implementar Plan A multi-cursor con territorios fijos

**Objetivo:** soportar múltiples cursores simultáneos sin territorios dinámicos.

Ejemplo 1: EST+LST con split fijo:

```text
cursor 0:
  direction = Forward
  territory = [horizon.start, midpoint)

cursor 1:
  direction = Backward
  territory = [midpoint, horizon.end)
```

Ejemplo 2: start + middle, misma dirección:

```text
cursor 0:
  direction = Forward
  territory = [horizon.start, midpoint)

cursor 1:
  direction = Forward
  territory = [midpoint, horizon.end)
```

### Importante

No hardcodear `midpoint`. El midpoint debe salir de config:

```rust
CursorTerritorySpec::FractionRange {
    start_fraction: 0.0,
    end_fraction: 0.5,
}
```

o:

```rust
Fixed {
    start: HorizonStart,
    end: Fraction(0.5),
}
```

### Tests

```text
multi_cursor_two_forward_fixed_territories_no_overlap
multi_cursor_forward_backward_fixed_territories_no_overlap
multi_cursor_forward_backward_both_cursors_contribute
multi_cursor_rejects_cross_territory_placement
multi_cursor_does_not_duplicate_task_across_cursors
```

### Acceptance criteria

* Dos cursores pueden colocar tareas en el mismo schedule.
* No hay overlaps.
* No hay duplicados.
* Cada cursor respeta su territorio.
* El algoritmo termina correctamente cuando un cursor queda exhausto.

---

# Milestone 9 — Integración pública sin romper EST/LST

**Objetivo:** exponer el nuevo algoritmo manteniendo compatibilidad.

En `schedulers/src/scheduler/mod.rs`, añadir módulo:

```rust
pub mod cursor;
```

y export:

```rust
pub use cursor::MultiCursorScheduler;
```

Mantener:

```rust
EstScheduler
LstScheduler
```

como APIs públicas.

Solo después de tener tests de equivalencia, opcionalmente:

```rust
EstScheduler::run(...)
  -> MultiCursorScheduler::single_forward(...)

LstScheduler::run(...)
  -> MultiCursorScheduler::single_backward(...)
```

Pero esto lo dejaría para el final. Al principio, el nuevo scheduler puede coexistir.

### Acceptance criteria

* Código externo que usa EST/LST sigue compilando.
* Nuevo scheduler disponible para tests y experimentos.
* Ninguna API pública existente desaparece.

---

# Milestone 10 — Integración en `lab` y sweep configs

**Objetivo:** poder lanzar experimentos con el nuevo scheduler.

Añadir nueva variante:

```rust
RunConfig::MultiCursor(MultiCursorRunConfig)
```

Actualmente los configs de sweep ya modelan variantes EST/HAP/LST. EST y LST comparten parámetros como `k_beams`, `branching_factor`, `endangered_threshold` y FOM. Esa compatibilidad debe mantenerse para el nuevo scheduler.

Config JSON sugerido:

```json
{
  "algorithm": "multi_cursor",
  "config": {
    "k_beams": 4,
    "branching_factor": 2,
    "endangered_threshold": 1,
    "cursor_policy": "best_candidate_global",
    "cursors": [
      {
        "id": "front",
        "direction": "forward",
        "territory": { "start": 0.0, "end": 0.5 }
      },
      {
        "id": "back",
        "direction": "backward",
        "territory": { "start": 0.5, "end": 1.0 }
      }
    ]
  }
}
```

También aliases cómodos:

```json
"layout": "est"
"layout": "lst"
"layout": "est_lst_split"
"layout": "start_mid_forward"
```

Pero internamente todos deberían expandirse a `MultiCursorConfig`.

### Acceptance criteria

* `lab run` puede ejecutar `algorithm = multi_cursor`.
* El registry guarda `algorithm = multi_cursor`.
* El slug de config distingue layouts y parámetros.
* EST/LST legacy siguen funcionando.

---

# Milestone 11 — Documentación, trazas y debugging

**Objetivo:** que el algoritmo sea interpretable.

Añadir logs:

```text
multi_cursor: starting scheduler — cursors=2, k_beams=..., branching_factor=...
multi_cursor: cursor=front refreshed — active=[..., ...], schedulable=...
multi_cursor: round=3 cursor=back candidate=1 placed task=42 at [...]
multi_cursor: cursor=front exhausted
multi_cursor: done — scheduled N tasks in R rounds
```

Actualizar README o docs internos:

```text
EST:
  single forward cursor

LST:
  single backward cursor / mirrored EST

MultiCursor:
  multiple fixed-territory cursors

Plan A:
  fixed territories

Future Plan B:
  dynamic territories
```

### Acceptance criteria

* Se puede entender qué cursor colocó cada tarea.
* Se documenta que Plan A usa territorios fijos.
* Se documenta explícitamente cómo se configuraría EST/LST con el nuevo modelo.

---

# Milestone 12 — Preparación explícita para Plan B

**Objetivo:** dejar puntos de extensión claros, sin implementar dinámico todavía.

Añadir tipos o stubs:

```rust
pub enum BoundaryRef {
    HorizonStart,
    HorizonEnd,
    Cursor(CursorId),
}

pub enum CursorTerritory {
    Fixed { start: Time<MJD>, end: Time<MJD> },
    Dynamic {
        left: BoundaryRef,
        right: BoundaryRef,
    },
}
```

Pero `Dynamic` puede devolver error controlado:

```rust
ScheduleError::UnsupportedConfiguration(
    "dynamic cursor territories are not implemented yet"
)
```

El motor debe estar estructurado para que Plan B solo requiera cambiar:

```rust
cursor.active_period(&state, horizon)
```

y no reescribir:

```text
- action collection
- child state building
- scoring
- validation
- beam pruning
```

### Acceptance criteria

* El código deja claro dónde irá Plan B.
* No hay lógica especial de midpoint dentro del beam search.
* El motor trabaja siempre con `active_period(...)`.

---

# Orden recomendado de implementación

```text
1. Milestone 0 — baseline tests
2. Milestone 1 — tipos
3. Milestone 2 — territorio activo
4. Milestone 3 — CandidateQueue extensions
5. Milestone 4 — MultiCursorState + CursorAction
6. Milestone 6 — single forward equivalence with EST
7. Milestone 7 — single backward equivalence with LST
8. Milestone 8 — fixed-territory multi-cursor
9. Milestone 10 — lab integration
10. Milestone 11 — docs/logs
11. Milestone 12 — Plan B readiness
```

Milestone 5, validación global, debe implementarse en paralelo con 4–8 porque es obligatorio para cualquier multi-cursor real.

---

# Riesgos principales

## 1. Equivalencia exacta con LST

LST actual depende del espejo temporal. No implementaría backward “a mano” al principio. Usaría `CursorFrame::Mirrored` para reproducir el comportamiento existente.

## 2. Duplicación de tareas entre colas

Cada cursor tendrá su propia cola. Por tanto, una tarea puede estar en varias colas. Hay que validar con:

```rust
schedule.contains(task_id)
```

antes de colocar.

## 3. Overlaps

Con varios cursores, el cursor ya no garantiza no-overlap. Hay que validar siempre contra el `Schedule` global usando `overlapping(...)`.

## 4. FOM context

El FOM se usa para puntuar hijos del beam search. El motor actual evalúa el score después de insertar el placement.  En multi-cursor hay que decidir cuidadosamente qué `FomContext` se pasa cuando hay varios cursores. Para la primera versión:

```text
ctx.cursor = cursor que acaba de avanzar
ctx.horizon = global horizon
ctx.possible_periods = original possible_periods
```

Pero para equivalencia exacta con LST single-backward, hay que testearlo. Si no coincide, se debe introducir un adapter específico para backward/single cursor.

---

# Definition of done global

El modelo está completo cuando se cumplen estas condiciones:

```text
1. EST legacy sigue pasando.
2. LST legacy sigue pasando.
3. MultiCursor(single forward) == EST legacy.
4. MultiCursor(single backward) == LST legacy.
5. MultiCursor(two cursors fixed territories) no produce overlaps.
6. MultiCursor no duplica tareas.
7. MultiCursor respeta dependencias de SchedulingBlock.
8. lab puede ejecutar sweeps multi_cursor.
9. El diseño contiene CursorTerritory::Fixed ahora y un punto limpio para Dynamic después.
```

Mi recomendación final: **no reemplazar EST/LST al principio**. Primero crear `MultiCursorScheduler` en paralelo, demostrar equivalencia y solo después convertir EST/LST en wrappers internos del nuevo motor. Esto reduce mucho el riesgo de romper resultados experimentales ya comparables.
