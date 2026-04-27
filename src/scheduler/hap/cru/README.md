Based on the foundation discussed in the previous section, we propose the CRU
heuristic algorithm to schedule a block of AOS. This section starts with an
overview and a high-level description of the algorithm, and follows it up with a
pseudo-code and a detailed explanation of its process.
The CRU algorithm takes an input schedule (sin ) which may have units
already planned, and a new scheduling (b) to add. As mentioned in Section
1.3.1, blocks can define multiple forms to be considered complete with different
priorities. As a result, the output of CRU is a set of schedules (Sout ) that show
how b can be completed in its different setups, with minimum loss of fitness in
sin .
As discussed in the previous section, CRU does not need to calculate the
fitness of the whole schedule, and works with the fitness loss of the manipulated
(moved or removed) units, which here is referred to as cost. The cost can be
calculated for any set of tasks and a schedule. The schedule determines the
current state of blocks’ completion, and based on that we can calculate the cost
of removing any set of tasks from it.
The cost of adding a task depends on where it is put, or in other words,
which of its windows (w ∈ Wt ) is selected. As a result, we calculate the cost for
individual windows (costw ), based on the units from the schedule that have to
be moved or removed (conflicted units, confw ), to fit the task. Based on these
details, a high-level flowchart of the CRU algorithm is depicted in Figure 3.1.
As described in Figure 3.1, CRU utilizes lobby, a set of tasks from which
the algorithm picks the ones to add to the schedule, using its Task Scheduling
Cycle. For each task in the input block b, the algorithm initiates lobby with
that task and starts the Task Scheduling, or inner Cycle. In the inner cycle,
tasks are removed from the lobby one by one, and put in the schedule, either in
an empty space, or by removing the conflicting units for the window with the
lowest cost, and placing the task there. Conflicting units that are removed from
the schedule are added to the lobby as tasks, and in the next iterations of the
inner cycle, they will be put back into the schedule. This swapping of tasks and
units with the goal of reducing the cost of the addition for the initial task of
the lobby (the one added from b), continues until at least one of the terminal
conditions of the inner cycle is met. There are two terminal conditions for the
Task Scheduling Cycle. One is reaching the maximum iterations defined by the
optimization parameter ιmax , and the other is having an empty lobby. An empty
lobby indicates that the addition of a new task is done without removing any
units from the schedule, and just by moving them around.
At each step in the Task Scheduling Cycle, the algorithm selects the window
of task that has the minimum cost. While this action generally leads the search
to lower overall costs, it allows increases in its steps, when even the minimum
option raises the overall cost. This enables the search to escape local minima
and does not get stuck. However, to make sure that the best result from the
inner cycles is not lost, CRU keeps the record of the schedule with the lowest
cost (calculated by the members of lobby), and saves is as slow to return at the
end to the Block Scheduling Cycle. The Task Scheduling Cycle avoids repetition
of moves to prevent the algorithm to get into a loop. In other words, if a task
is placed in the schedule from the lobby, it cannot be removed again during the
same iterations of the inner cycle.
With the Task Scheduling Cycle, CRU can accumulate different tasks of b to
satisfy its constraints, without taking into account its priority (added fitness).
For example, considering the block b : {t1 ∧t2 }∨{t3 }, CRU proceeds as follows:
1. Adds t1 to the input schedule.
2. Adds t2 to the schedule from the previous step.
3. Adds the schedule from the previous step (t1 ∧ t2 ) to Sout .
4. Adds t3 to the input schedule.
5. Adds the schedule from the previous step (t3 ) to Sout .
In the example, at the end of CRU, the set of output schedules, Sout , contains
two schedules which correlate to the two ways that we can complete (satisfy
the constraints of) b. These schedules could be represented as s1 = {u1 , u2 },
containing the units of the first two tasks, and s2 = {u3 } with just a unit of
t3 . The high-level description of CRU facilitates understanding of the detailed
algorithm, which is presented as pseudo-code in Algorithm 1.
According to the Algorithm 1, CRU takes schedule sin and block b as input,
and starts with the initialization of slow and Sout lines 1 and 2, respectively. In
line 3, CRU enters the upper cycle or the Block Scheduling Cycle which while
there are tasks to be processed in b, iterates on them. Inside the cycle, after
assigning an empty set to lobby in line 4, the RetrieveTask function adds an
unprocessed task to it in line 5, according to the block constraints (Cb ). Based
on the retrieved task, the algorithm fetches the correct schedule, between the
input or the results from the previous searches, as the base of the inner cycle or
the Task Scheduling Cycle as sbase , in line 6. Lines 7 and 8 reset the values of
iter and costmin to prepare the algorithm for the inner cycle. The inner cycle,
which spans from lines 9 to 34, has the responsibility to add the task from lobby
into its base schedule. The terminal conditions of this cycle are indicated in line
9. The TakeTask function, in line 10, removes a member of lobby and returns it
as tin . The set costs is assigned an empty set, and the variable to hold the cost
of each window, costw , is initialized with a non-zero value to allow the algorithm
to enter the while loop between lines 13 and 18. This loops goes through every
valid window of the task tin to find the best place to add it to the schedule.
The RetrieveWindow function returns a valid and unprocessed window of tin as
w, from its set of windows Wtin , and determined by the task constraints Ctin .
Line 15 of Algorithm 1, finds the units in sbase that have conflict with tin in the
window w, using the FindConflicts function. Here, conflict means that a unit
is overlapping with w or its existence break a constraint of b or tin . In other
words, the units in the schedule that need to be removed to fit tin at w, in
sbase . The set of conflicted units for w (confw ) is used in line 16 to calculate
the cost of adding tin at w, and saves it as costw . This cost is added to the
set of costs to be utilized later, once every window is processed or a window
with zero cost is found (costw == 0). The minimum value of the cost is zero,
and if the algorithm finds such a window, there is no point in further searching;
otherwise, the cost of all windows must be calculated. If the algorithm exits
the while loop in line 18 with a zero cost window, it adds tin to sbase at the
window w, as the unit uw
tin . That is indicated in lines 19 to 21. If there are no
such windows, CRU finds the window with the minimum cost using the function
MinimumCostWindow, with costs and Wtin as input, and saves it in win (line
23). Lines 24 to 26, remove the conflicts of win from sbase , adds them to lobby,
in and put tin in sbase at the window w, as unit uw tin .
Once the addition of tin is done by one way or the other, the algorithm
calculates, in line 28, the cost of the updated lobby to determine if it’s below
the best one we found during the current Task Scheduling Cycles. If the cost
of the current lobby (costlobby ) is indeed below the minimum cost found, we
update costmin and slow , as represented in lines 29 to 32. The only remaining
action in the inner cycle is to increase the iteration counter (iter) in line 33.
By the end of the Task Scheduling Cycle, in lines 35 to 37, CRU checks if the
addition of a new task from b to sin completes it, according to its constraints
(Cb ). If the conditions are satisfied, slow is added to the output set. Once all
the tasks and constraints of b are processed by the Task Scheduling Cycle, CRU
terminates and returns Sout , as indicated in line 39 of Algorithm 1.
The proposed Conflict Resolution Unit (CRU) algorithm generally assumes
that the input schedule is good (regarding fitness) and focuses its search on the
different neighborhoods around it without exhausting one. It concentrates on
local exploitation of neighborhoods, while allowing some exploration using its
escape local minima mechanism.
The proposed CRU fulfills all the sub-goals defined for the heuristic research,
as detailed in Section 3.2. In addition to its quick escape mechanism and the use
of cost calculations (instead of fitness), which contributes to a low computational
cost, CRU bases its moves on the specific structure of AOS and the smallest
transitions (regarding fitness fluctuation) in the solution space. There are three
functions in the algorithm that handle the block constraints (Cb ), RetrieveTask,
RetrieveSchedule, and SchedulingBlockCompletes. In contrast, the is only one
function to handle task constraints (Ct ), which is RetrieveWindow.
RetrieveTask and RetrieveSchedule functions use Cb to direct CRU to cor-
rectly build optimized schedules for the input block, and the SchedulingBlock-
Completes function checks their validity. On the other hand, the RetrieveWindow
uses Ct to pass the algorithm the correct possible windows where the task can
be scheduled. All these functions are generally low-cost computationally, and
enable the algorithm to adapt to a large variety of common tasks and block
constraints, and apply them individually, without the need to change any other
part. This feature further aligns the proposal with the sub-goals of the research,
defined in 3.3.1.