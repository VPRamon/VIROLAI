## Plan de implementación

### 1. Crear un nuevo módulo `lst`

Añadir una nueva carpeta:

```text
src/scheduler/lst/
```

Con una estructura mínima:

```text
src/scheduler/lst/
  mod.rs
  algorithm.rs
  transform.rs
  fom.rs        // opcional, si queremos FOM temporalmente correcto
  tests.rs
```

El objetivo es que `lst` no duplique la lógica de `beam`, `candidate`, `queue` ni `ordering`. Debe delegar en `EstScheduler`.

---

### 2. Definir `LstScheduler`

Crear un scheduler equivalente a `EstScheduler`, pero internamente usa EST sobre ventanas reflejadas.

```rust
#[derive(Debug, Clone)]
pub struct LstScheduler {
    pub est: EstScheduler,
}
```

Constructores recomendados:

```rust
impl LstScheduler {
    pub fn new(config: est::Configuration) -> Result<Self, ScheduleError> {
        Ok(Self {
            est: EstScheduler::new(config)?,
        })
    }

    pub fn with_fom(
        config: est::Configuration,
        fom: Arc<dyn ScheduleFom>,
    ) -> Result<Self, ScheduleError> {
        Ok(Self {
            est: EstScheduler::with_fom(config, fom)?,
        })
    }
}
```

Esto mantiene los mismos parámetros:

```text
k_beams
branching_factor
endangered_threshold
fom
```

El EST actual ya centraliza esos parámetros en `Configuration` y `ScheduleFom`. 

---

### 3. Implementar reflexión temporal

En `src/scheduler/lst/transform.rs`, añadir funciones puras.

Regla base:

```text
mirror(t) = horizon.start + horizon.end - t
```

Para un periodo:

```text
[start, end) -> [mirror(end), mirror(start))
```

Código aproximado:

```rust
use crate::prescheduler::TaskPeriodMap;
use crate::schedule::{Schedule, TaskPlacement};
use crate::time::{MJD, Period, PeriodSet, TaskId, Time};

pub fn mirror_time(t: Time<MJD>, horizon: &Period<MJD>) -> Time<MJD> {
    Time::<MJD>::new(horizon.start.value() + horizon.end.value() - t.value())
}

pub fn mirror_period(period: &Period<MJD>, horizon: &Period<MJD>) -> Period<MJD> {
    Period::new(
        mirror_time(period.end, horizon),
        mirror_time(period.start, horizon),
    )
}

pub fn mirror_period_set(set: &PeriodSet<MJD>, horizon: &Period<MJD>) -> PeriodSet<MJD> {
    PeriodSet::from_periods(
        set.iter()
            .map(|period| mirror_period(period, horizon))
            .collect(),
    )
}

pub fn mirror_task_periods(
    possible_periods: &TaskPeriodMap,
    horizon: &Period<MJD>,
) -> TaskPeriodMap {
    possible_periods
        .iter()
        .map(|(&task_id, periods)| {
            (task_id, mirror_period_set(periods, horizon))
        })
        .collect()
}
```

`PeriodSet::from_periods` normaliza y ordena los periodos, así que aunque la reflexión invierta el orden, el resultado vuelve a quedar canónico.

---

### 4. Ejecutar EST sobre ventanas reflejadas

En `LstScheduler::run_scheduler`:

```rust
pub fn run_scheduler(
    &self,
    tasks: &[Task],
    possible_periods: &TaskPeriodMap,
    horizon: &Period<MJD>,
) -> Result<Schedule, ScheduleError> {
    let mirrored_periods = transform::mirror_task_periods(possible_periods, horizon);

    let mirrored_schedule = self.est.run_scheduler(
        tasks,
        &mirrored_periods,
        horizon,
    )?;

    Ok(transform::unmirror_schedule(&mirrored_schedule, horizon))
}
```

El EST actual construye una `CandidateQueue`, crea un `ScheduleState` con cursor en `horizon.start` y llama a `beam::run_search`. Al reflejar las ventanas, ese avance hacia delante equivale a programar hacia atrás en el tiempo original. 

---

### 5. Deshacer la reflexión del `Schedule`

Una colocación reflejada:

```text
[start', end')
```

se convierte en:

```text
[mirror(end'), mirror(start'))
```

Código aproximado:

```rust
pub fn unmirror_schedule(
    mirrored: &Schedule,
    horizon: &Period<MJD>,
) -> Schedule {
    let mut out = Schedule::new();

    for placement in mirrored.placements() {
        let start = mirror_time(placement.end.to::<MJD>(), horizon);
        let end = mirror_time(placement.start.to::<MJD>(), horizon);

        out.insert_placement(TaskPlacement {
            task_id: placement.task_id,
            start,
            end,
            block_id: placement.block_id,
        });
    }

    out
}
```

---

### 6. Añadir exports públicos

En `src/scheduler/mod.rs`, añadir:

```rust
pub mod lst;
```

En `src/scheduler/lst/mod.rs`:

```rust
mod algorithm;
mod transform;

pub use algorithm::{LstScheduler, run_scheduler};
```

También puedes exportar `mirror_*` solo para tests:

```rust
#[cfg(test)]
pub(crate) mod transform;
```

---

### 7. Decidir cómo tratar `run_with_problem`

Primera versión recomendada: **implementar solo `run_scheduler` sin bloques**.

Motivo: las dependencias cambian de dirección al reflejar el tiempo.

Si en el problema original tienes:

```text
A -> B
```

es decir, `A` debe ir antes que `B`, en el espacio reflejado necesitas:

```text
B' -> A'
```

Por tanto, `run_with_problem` no se puede envolver correctamente sin transformar también los `SchedulingBlock`.

Plan para `run_with_problem`, fase posterior:

1. Crear una función `mirror_blocks`.
2. Para cada dependencia `from -> to`, crear `to -> from`.
3. Ejecutar `est.run_with_problem` con bloques invertidos.
4. Deshacer el schedule.

Pero antes conviene corregir el chequeo actual de dependencias del EST domain-aware, porque ahora usa el orden topológico como si todos los nodos anteriores fueran predecesores reales. Ese comportamiento puede rechazar planificaciones válidas con tareas no relacionadas.

---

### 8. Adaptar el FOM si hay soft constraints dependientes del tiempo

Primera versión simple: reutilizar `SoftConstraintFom`.

Pero si una soft constraint depende del instante de inicio, hay un problema: EST puntuará usando el tiempo reflejado, no el tiempo real original.

Solución robusta: crear un wrapper FOM:

```rust
pub struct MirroredFom {
    inner: Arc<dyn ScheduleFom>,
    horizon: Period<MJD>,
}
```

Su `evaluate` debería:

1. Deshacer temporalmente el schedule reflejado.
2. Evaluar `inner` sobre el schedule original.

Conceptualmente:

```rust
impl ScheduleFom for MirroredFom {
    fn evaluate(&self, schedule: &Schedule, tasks: &[Task]) -> f64 {
        let original = unmirror_schedule(schedule, &self.horizon);
        self.inner.evaluate(&original, tasks)
    }
}
```

Esto hace que la poda del beam search compare estados según la calidad real en tiempo original.

Recomendación: implementar esto desde el principio si quieres que LST sea “exactamente equivalente” y no solo una aproximación estructural.

---

### 9. Añadir CLI

Ahora el binario `scheduler` solo acepta flags EST como `--est-fom`, `--est-e`, `--est-k`, `--est-b`. 

Añadir un selector:

```bash
--algorithm est|lst
```

Por defecto:

```text
est
```

Ejemplo:

```bash
cargo run --bin scheduler -- data/CTA-N/scheduling_problem.json \
  --algorithm lst \
  --est-fom soft_constraint \
  --est-e 2 \
  --est-k 5 \
  --est-b 3
```

Aunque los flags sigan llamándose `--est-*`, internamente pueden mapearse a `Configuration`, compartida por ambos algoritmos.

Más adelante podrías renombrarlos a flags neutros:

```text
--scheduler-k
--scheduler-b
--scheduler-endangered-threshold
```

Pero no lo haría en la primera iteración para evitar romper compatibilidad.

---

### 10. Tests mínimos

Añadir tests en `src/scheduler/lst/tests.rs`.

#### Test 1: reflexión de periodo

```text
horizon = [0, 10)
period = [2, 4)
mirror = [6, 8)
```

#### Test 2: doble reflexión conserva ventana

```text
mirror(mirror(period)) == period
```

#### Test 3: LST coloca al final de una ventana simple

Tarea:

```text
duration = 1
window = [0, 10)
horizon = [0, 10)
```

Resultado esperado:

```text
start = 9
end = 10
```

#### Test 4: múltiples ventanas

Tarea:

```text
duration = 1
windows = [0, 2), [5, 8)
```

Resultado esperado:

```text
start = 7
end = 8
```

#### Test 5: dos tareas secuenciales sin solape

Dos tareas de duración `1`, ambas con ventana `[0, 10)`.

LST greedy debería producir algo equivalente a:

```text
task_1: [8, 9)
task_2: [9, 10)
```

o el orden correspondiente según el ordering reflejado.

#### Test 6: equivalencia con EST reflejado

Verificar que:

```text
LST(original_windows)
==
unmirror(EST(mirror(original_windows)))
```

Este test valida directamente el contrato del wrapper.

#### Test 7: FOM temporal

Si hay soft constraints dependientes del tiempo, comprobar que `MirroredFom` evalúa usando el tiempo original y no el reflejado.

---

### 11. QA final

Ejecutar:

```bash
cargo fmt --all
cargo clippy --all-targets -- -D warnings
cargo test --all-features
```

El repo documenta esos comandos como requisitos mínimos de QA.

## Orden recomendado de trabajo

1. Implementar `lst/transform.rs`.
2. Implementar `LstScheduler::run_scheduler`.
3. Añadir exports.
4. Añadir tests de transformación y schedule.
5. Añadir CLI `--algorithm est|lst`.
6. Implementar `MirroredFom`.
7. Añadir tests de FOM.
8. Dejar `run_with_problem` para una segunda fase con inversión de dependencias.
