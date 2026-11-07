/** UE class handle. Generated declarations merge this global shape for ergonomic `Class<T>` use. */
/** Vendored KDataForgeJS runtime declaration. No package manager or node_modules required. */
declare module "@sf-kmods/kdataforgejs" {
    export enum TypeKind {
        Value = 0,
        Struct = 1,
        Object = 2,
        Class = 3,
        Enum = 4,
    }
    export enum MemberAccess {
        None = 0,
        Read = 1,
        Write = 2,
        ReadWrite = 3,
    }
    export enum InvocationKind {
        None = 0,
        Static = 1,
        Member = 2,
        Custom = 3,
    }
    export enum CallEffect {
        None = 0,
        ReadOnly = 1,
        ReversibleMutation = 2,
        IrreversibleMutation = 3,
    }
    export enum TargetKind {
        Realm = 0,
        Class = 1,
        CDO = 2,
        Actor = 3,
        Behavior = 4,
    }
    export enum TargetCardinality {
        One = 0,
        Many = 1,
    }
    export enum Authority {
        Server = 0,
        ClientCosmetic = 1,
        Both = 2,
    }
    export enum ThreadAffinity {
        GameThreadOnly = 0,
        AnyThread = 1,
    }
    export enum LifecyclePhase {
        Initialization = 0,
        WorldReady = 1,
        Runtime = 2,
    }
    export enum CompareOperator {
        Equal = "equals",
        EqualSymbol = "==",
        NotEqual = "notEquals",
        NotEqualSymbol = "!=",
        Less = "less",
        LessSymbol = "<",
        LessOrEqual = "lessOrEqual",
        LessOrEqualSymbol = "<=",
        Greater = "greater",
        GreaterThan = "greaterThan",
        GreaterSymbol = ">",
        GreaterOrEqual = "greaterOrEqual",
        GreaterOrEqualSymbol = ">=",
    }
    export enum ImportKind {
        Text = "text",
        Yaml = "yaml",
        Binary = "binary",
        Image = "image",
        Audio = "audio",
    }
    export enum HookPhase {
        Before = "before",
        After = "after",
        Replace = "replace",
    }
    export enum ConditionBehavior {
        And = "AND",
        Or = "OR",
    }
    export enum TagMatch {
        All = "all",
        Any = "any",
    }
    export enum ConditionOutcome {
        Match = "match",
        NoMatch = "noMatch",
        Deferred = "deferred",
        Invalid = "invalid",
    }
    export enum EventScalarType {
        Boolean = "boolean",
        Int64 = "int64",
        Number = "number",
        String = "string",
        UInt64 = "uint64",
    }
    export enum NumericSign {
        Negative = -1,
        Positive = 1,
    }
    export enum Ordering {
        Less = -1,
        Equal = 0,
        Greater = 1,
    }
    export enum SemVerRelease {
        Major = "major",
        Minor = "minor",
        Patch = "patch",
        Prerelease = "prerelease",
    }
    export enum ResourceForm {
        Solid = "solid",
        Liquid = "liquid",
        Gas = "gas",
    }
    export enum StackSize {
        One = "one",
        Small = "small",
        Medium = "medium",
        Big = "big",
        Huge = "huge",
        Fluid = "fluid",
    }
    export enum ContentKind {
        Item = "item",
        Recipe = "recipe",
    }
    export enum DiagnosticSeverity {
        Info = "info",
        Warning = "warning",
        Error = "error",
    }
    export enum DeliveryMode {
        Immediate = "immediate",
        Debounce = "debounce",
        Throttle = "throttle",
    }
    export enum ActorChangeKind {
        Added = "added",
        Removed = "removed",
    }
    export enum AsyncResultStatus {
        Fulfilled = "fulfilled",
        Rejected = "rejected",
    }
    export enum LifecycleEvent {
        WorldReady = "worldReady",
        Travel = "travel",
        Shutdown = "shutdown",
    }
    export enum DocumentType {
        Asset = "asset",
        CategoryReorder = "category_reorder",
        Cdo = "cdo",
        Class = "class",
        Config = "config",
        Curve = "curve",
        Localization = "localization",
        Item = "item",
        Resource = "resource",
        Building = "building",
        Unlock = "unlock",
        Recipe = "recipe",
        Schematic = "schematic",
        Research = "research",
        Mam = "mam",
        Sound = "sound",
        Sublevel = "sublevel",
        DataAsset = "dataasset",
        EventRule = "eventrule",
        GameTag = "gametag",
        ResourceNode = "resourcenode",
        SinkPoints = "sinkpoints",
    }
    export enum CurveType {
        Float = "float",
        Vector = "vector",
    }
    export enum CategoryKind {
        Items = "items",
        Recipe = "recipe",
        Schematic = "schematic",
        Settings = "settings",
    }
    export enum ActorAction {
        Destroy = "destroy",
        Dismantle = "dismantle",
    }
    export enum ClassScope {
        Any = "any",
        Blueprint = "blueprint",
        Native = "native",
    }
    export enum PropertyOperation {
        Add = "add",
        Append = "append",
        AppendUnique = "append_unique",
        Clamp = "clamp",
        Clear = "clear",
        Copy = "copy",
        Divide = "divide",
        Duplicate = "duplicate",
        Insert = "insert",
        Max = "max",
        Min = "min",
        Move = "move",
        Multiply = "multiply",
        Prepend = "prepend",
        Remove = "remove",
        RemoveAt = "remove_at",
        ReplaceRefs = "replace_refs",
        Replace = "replace",
        Reverse = "reverse",
        Set = "set",
        Sort = "sort",
        Subtract = "subtract",
        Swap = "swap",
        Unique = "unique",
        UpdateWhere = "update_where",
    }
    export enum ConfigFieldKind {
        Text = "text",
        Number = "number",
        Boolean = "boolean",
        Select = "select",
    }
    export enum ConfigApplyMode {
        Hot = "hot",
        Cold = "cold",
    }
    export enum ConfigValueType {
        String = "string",
        Text = "text",
        Name = "name",
        Tag = "tag",
        Int8 = "int8",
        Int16 = "int16",
        Int32 = "int32",
        Int64 = "int64",
        Uint8 = "uint8",
        Float = "float",
        Double = "double",
        Bool = "bool",
    }
    export enum NumberDisplayMode {
        Slider = "slider",
        Input = "input",
    }
    export enum CurveExtrapolation {
        None = "none",
        Constant = "constant",
        Linear = "linear",
        Cycle = "cycle",
        CycleWithOffset = "cycleWithOffset",
        Oscillate = "oscillate",
    }
    export enum CurveInterpolation {
        Constant = "constant",
        Linear = "linear",
        Cubic = "cubic",
    }
    export enum CurveTangentMode {
        Auto = "auto",
        SmartAuto = "smartAuto",
        User = "user",
        Break = "break",
    }
    export enum CurveTangentWeightMode {
        None = "none",
        Arrive = "arrive",
        Leave = "leave",
        Both = "both",
    }
    export enum EventRuleEvent {
        WorldReady = "worldReady",
        SchematicPurchased = "schematicPurchased",
    }
    export enum ResourcePurity {
        Impure = "Impure",
        Normal = "Normal",
        Pure = "Pure",
    }
    export enum ResourceNodeKind {
        Node = "Node",
        FrackingSatellite = "FrackingSatellite",
        FrackingCore = "FrackingCore",
        Geyser = "Geyser",
        Deposit = "Deposit",
    }
    export enum SinkTrack {
        Default = "Default",
        Exploration = "Exploration",
    }
    export enum Capability {
        SchematicsRead = "kdf.schematics.read",
        OverridesTemporary = "kdf.overrides.temporary",
        FilesWrite = "kdf.files.write",
        DatatablesWrite = "kdf.datatables.write",
        DatatablesRead = "kdf.datatables.read",
        BehaviorRuntime = "kdf.behavior.runtime",
        ClassesDefine = "kdf.classes.define",
        ConfigRead = "kdf.config.read",
        ConditionsEvaluate = "kdf.conditions.evaluate",
        DiagnosticsWrite = "kdf.diagnostics.write",
        DocumentsApply = "kdf.documents.apply",
        EventsCustom = "kdf.events.custom",
        ImportsRead = "kdf.imports.read",
        ItemsDefine = "kdf.items.define",
        ItemsRead = "kdf.items.read",
        NativeCall = "kdf.native.call",
        NativeHook = "kdf.native.hook",
        NativeRead = "kdf.native.read",
        NativeWrite = "kdf.native.write",
        RecipesDefine = "kdf.recipes.define",
        RecipesRead = "kdf.recipes.read",
        ReflectionCall = "kdf.reflection.call",
        ReflectionRead = "kdf.reflection.read",
        ReflectionWrite = "kdf.reflection.write",
        ScriptAdmin = "kdf.script.admin",
        WorldRead = "kdf.world.read",
    }
    /** Capability ids for `defineMeta`. Reading and writing reflected values is always granted. */
    export interface CapabilityMap {
        readonly [Capability.SchematicsRead]: true;
        readonly [Capability.OverridesTemporary]: true;
        readonly [Capability.FilesWrite]: true;
        readonly [Capability.DatatablesWrite]: true;
        readonly [Capability.DatatablesRead]: true;
        readonly [Capability.BehaviorRuntime]: true;
        readonly [Capability.ClassesDefine]: true;
        readonly [Capability.ConfigRead]: true;
        readonly [Capability.ConditionsEvaluate]: true;
        readonly [Capability.DiagnosticsWrite]: true;
        readonly [Capability.DocumentsApply]: true;
        readonly [Capability.EventsCustom]: true;
        readonly [Capability.ImportsRead]: true;
        readonly [Capability.ItemsDefine]: true;
        readonly [Capability.ItemsRead]: true;
        readonly [Capability.NativeCall]: true;
        readonly [Capability.NativeHook]: true;
        readonly [Capability.NativeRead]: true;
        readonly [Capability.NativeWrite]: true;
        readonly [Capability.RecipesDefine]: true;
        readonly [Capability.RecipesRead]: true;
        readonly [Capability.ReflectionCall]: true;
        readonly [Capability.ReflectionRead]: true;
        readonly [Capability.ReflectionWrite]: true;
        readonly [Capability.ScriptAdmin]: true;
        readonly [Capability.WorldRead]: true;
    }
    export type CapabilityId = keyof CapabilityMap & string;
    export interface Meta { readonly capabilities: readonly CapabilityId[]; }
    /** The current script realm's shared context. Import it in entry points and helper modules. */
    export const ctx: Context;
    export type Main = (ctx: Context) => Promise<void>;
    export function defineMeta(meta: Meta): Readonly<Meta>;
    export function defineHandler(handler: (ctx: Context) => Promise<void>): (ctx: Context) => Promise<void>;
    export function defineBehavior<T extends object>(behavior: T): Readonly<T>;
    export function all(values: readonly unknown[]): boolean;
    export function any(values: readonly unknown[]): boolean;

    /**
     * Scalar neutral values accepted by property writes, calls, and comparisons.
     * TypeScript cannot range-check primitive bigint values. Runtime accepts signed bigint values
     * from -9223372036854775808n through 9223372036854775807n and unsigned bigint values
     * 0n through 18446744073709551615n; values outside those ranges are rejected at runtime.
     */
    export type InputValue = null | undefined | boolean | bigint | number | string;
    /** Scalar neutral values returned by property reads and calls. */
    export type Value = null | boolean | bigint | number | string;

    export interface ExecutableDescriptor { readonly providerId: string; readonly adapterId: string; readonly targetId: string; readonly targetKind: TargetKind; readonly authority: Authority; readonly threadAffinity: ThreadAffinity; readonly lifecyclePhase: LifecyclePhase; }
    export interface TypeDescriptor { readonly id: string; readonly kind: TypeKind; readonly baseTypeId: string; readonly capabilities: readonly string[]; readonly providerId: string; readonly nativeTypeScriptName: string; }
    export interface PropertyDescriptor extends ExecutableDescriptor { readonly id: string; readonly ownerTypeId: string; readonly valueTypeId: string; readonly access: MemberAccess; readonly capabilities: readonly string[]; readonly nativeMemberName: string; readonly nativeContainerName: string; }
    export interface FunctionDescriptor extends ExecutableDescriptor { readonly id: string; readonly ownerTypeId: string; readonly returnTypeId: string; readonly invocation: InvocationKind; readonly callEffect: CallEffect; readonly parameterTypeIds: readonly string[]; readonly capabilities: readonly string[]; readonly nativeMemberName: string; }
    export interface EventFieldDescriptor { readonly name: string; readonly valueTypeId: string; readonly optional: boolean; }
    export interface EventDescriptor extends ExecutableDescriptor { readonly id: string; readonly ownerTypeId: string; readonly returnTypeId: string; readonly payloadTypeIds: readonly string[]; readonly payloadFields: readonly EventFieldDescriptor[]; readonly capabilities: readonly string[]; }
    export interface ImportDescriptor extends ExecutableDescriptor { readonly id: string; readonly valueTypeId: string; readonly capabilities: readonly string[]; }
    export interface TargetDescriptor { readonly id: string; readonly declaredTypeId: string; readonly providerId: string; readonly kind: TargetKind; readonly cardinality: TargetCardinality; readonly authority: Authority; readonly threadAffinity: ThreadAffinity; readonly lifecyclePhase: LifecyclePhase; readonly nativeClassPath: string; }
    export interface TrackedDescriptor extends ExecutableDescriptor { readonly id: string; readonly ownerTypeId: string; readonly valueTypeId: string; readonly targetId: string; readonly access: MemberAccess; readonly capabilities: readonly string[]; }
    export interface NativeAdapterDescriptor { readonly providerId: string; readonly adapterId: string; readonly families: number; }
    export interface Catalog { readonly token: string; readonly generation: bigint; readonly digest: string; readonly types: readonly TypeDescriptor[]; readonly properties: readonly PropertyDescriptor[]; readonly functions: readonly FunctionDescriptor[]; readonly events: readonly EventDescriptor[]; readonly imports: readonly ImportDescriptor[]; readonly targets: readonly TargetDescriptor[]; readonly tracked: readonly TrackedDescriptor[]; readonly nativeAdapters: readonly NativeAdapterDescriptor[]; }

    const targetBrand: unique symbol; const valueRefBrand: unique symbol; const subscriptionBrand: unique symbol;
    const actorBrand: unique symbol; const actorClassBrand: unique symbol; const hookBrand: unique symbol;
    export interface Target { readonly [targetBrand]: never; }
    export interface UObject extends Target { }
    export interface Class<T extends UObject = UObject> extends Target {
        readonly __kdfClassBrand: never;
        readonly path: string;
        getClassDefaultObject(): T;
    }
    export interface NativeClassPathMap { }
    export interface LookupOptions { readonly checked?: boolean; }
    export interface CheckedLookupOptions { readonly checked: true; }
    export type ClassValue<C extends Class<UObject>> = C extends Class<infer T> ? T : never;
    export type ExactType<A, B> = [A] extends [B] ? ([B] extends [A] ? true : false) : false;
    export type NativeClassPathFor<C extends Class<UObject>> = {
        readonly [P in keyof NativeClassPathMap & string]:
            ExactType<NativeClassPathMap[P], ClassValue<C>> extends true ? P : never;
    }[keyof NativeClassPathMap & string];
    export interface ClassSearchParams {
        readonly pathPrefix?: string;
        readonly pathContains?: string;
        readonly namePrefix?: string;
        readonly nameContains?: string;
        readonly exactName?: string;
        readonly exactClass?: boolean;
        readonly caseSensitive?: boolean;
        readonly includeNative?: boolean;
        readonly includeBlueprints?: boolean;
        readonly limit?: number;
    }
    export interface Realm extends Target {
        readonly counter: WritableValueRef<bigint>;
        readonly uint64_max: ReadonlyValueRef<bigint>;
    }
    /** Read-only recursive containers avoid circular aliases in the bundled TypeScript compiler. */
    export interface StructuredInputArray extends ReadonlyArray<StructuredInputValue> { }
    export interface StructuredInputObject { readonly [key: string]: StructuredInputValue; }
    export interface StructuredInputMap {
        readonly size: number; get(key: StructuredInputValue): StructuredInputValue | undefined;
        has(key: StructuredInputValue): boolean; entries(): IterableIterator<[StructuredInputValue, StructuredInputValue]>;
        keys(): IterableIterator<StructuredInputValue>; values(): IterableIterator<StructuredInputValue>;
        forEach(callback: (value: StructuredInputValue, key: StructuredInputValue) => void, thisArg?: unknown): void;
        [Symbol.iterator](): IterableIterator<[StructuredInputValue, StructuredInputValue]>;
    }
    export interface StructuredInputSet {
        readonly size: number; has(value: StructuredInputValue): boolean; entries(): IterableIterator<[StructuredInputValue, StructuredInputValue]>;
        keys(): IterableIterator<StructuredInputValue>; values(): IterableIterator<StructuredInputValue>;
        forEach(callback: (value: StructuredInputValue, value2: StructuredInputValue) => void, thisArg?: unknown): void;
        [Symbol.iterator](): IterableIterator<StructuredInputValue>;
    }
    /** Bounded structured values accepted by target resolution, tracked CAS, and actor writes. */
    export type StructuredInputValue = InputValue | Target | StructuredInputArray | StructuredInputObject | StructuredInputMap | StructuredInputSet;
    export interface StructuredArray extends ReadonlyArray<StructuredValue> { }
    export interface StructuredObject { readonly [key: string]: StructuredValue; }
    export interface StructuredMap {
        readonly size: number; get(key: StructuredValue): StructuredValue | undefined;
        has(key: StructuredValue): boolean; entries(): IterableIterator<[StructuredValue, StructuredValue]>;
        keys(): IterableIterator<StructuredValue>; values(): IterableIterator<StructuredValue>;
        forEach(callback: (value: StructuredValue, key: StructuredValue) => void, thisArg?: unknown): void;
        [Symbol.iterator](): IterableIterator<[StructuredValue, StructuredValue]>;
    }
    export interface StructuredSet {
        readonly size: number; has(value: StructuredValue): boolean; entries(): IterableIterator<[StructuredValue, StructuredValue]>;
        keys(): IterableIterator<StructuredValue>; values(): IterableIterator<StructuredValue>;
        forEach(callback: (value: StructuredValue, value2: StructuredValue) => void, thisArg?: unknown): void;
        [Symbol.iterator](): IterableIterator<StructuredValue>;
    }
    /** Bounded structured values returned by tracked reads and authorized content imports. */
    export type StructuredValue = Value | Target | StructuredArray | StructuredObject | StructuredMap | StructuredSet;
    export interface ReadonlyValueRef<T = Value> { readonly [valueRefBrand]: never; get(): T; }
    export interface WritableValueRef<T = Value> extends ReadonlyValueRef<T> { set(value: T): void; }
    export type ValueRef<T = Value> = ReadonlyValueRef<T> | WritableValueRef<T>;
    export interface Subscription { readonly [subscriptionBrand]: never; unsubscribe(): void; }
    export interface Actor { readonly [actorBrand]: never; }
    export type CatalogActor<K extends string> = Actor & { readonly [actorClassBrand]: K };
    /** Pack-generated declarations augment these maps with closed catalog literals. */
    export interface ActorClassMap { }
    export interface ActorPropertyMap { }
    export type ActorClassKey = keyof ActorClassMap & string;
    export type ActorInitialWrites<K extends ActorClassKey> = Partial<ActorPropertyMap[K]>;
    export type ActorPropertyKey<K extends ActorClassKey> = keyof ActorPropertyMap[K] & string;
    export type ActorPropertyValue<K extends ActorClassKey, P extends ActorPropertyKey<K>> = ActorPropertyMap[K][P];
    /** Named events receive one frozen object; positional events receive each value as a callback argument. */
    export interface EventArgs { readonly [field: string]: Value | undefined; }
    export type NamedEventCallback = (args: EventArgs) => void;
    export type PositionalEventCallback = (value: Value, ...values: readonly Value[]) => void;
    /** Typed handle to one hookable function of a class; `Class.hooks.<Name>` is the only way to obtain one. */
    export interface Hook<TTarget extends UObject = UObject, TArgs extends readonly Value[] = readonly Value[], TReturn = void> {
        readonly [hookBrand]: never;
        readonly __target?: TTarget; readonly __args?: TArgs; readonly __return?: TReturn;
    }
    /** A Blueprint-assignable delegate of an object. Binding a key again replaces that binding. */
    export interface Delegate<TArgs extends readonly Value[]> {
        bind(key: string, callback: (...args: TArgs) => void): Subscription;
        unbind(key: string): void;
    }
    /** What a hook callback receives: the receiving object, the call arguments and, after the call, its return value. */
    export interface HookCall<TTarget extends UObject, TArgs extends readonly Value[], TReturn> {
        readonly target?: TTarget; readonly args: TArgs; readonly returnValue?: TReturn;
    }
    export interface BehaviorHandlerDescriptor { readonly eventId: string; readonly callbackToken: string; readonly payloadTypeIds: readonly string[]; }
    export interface BehaviorDescriptor { readonly id: string; readonly baseTypeId: string; readonly maxInstances: number; readonly composedBehaviorIds?: readonly string[]; readonly handlers: readonly BehaviorHandlerDescriptor[]; readonly authority?: Authority; readonly threadAffinity?: ThreadAffinity; }
    export interface BehaviorInvocation { readonly eventId: string; readonly target: Actor; readonly payload: Readonly<Record<string, Value>> | readonly Value[]; }
    export type BehaviorHandler = (invocation: BehaviorInvocation) => unknown;
    export interface BehaviorRegistration { readonly id: string; }
    export interface BehaviorInstance { readonly __kdfBehaviorInstance: never; }
    export interface TickSubscription extends Subscription { }
    export interface ModMeta {
        readonly reference: string; readonly name: string; readonly version: string; readonly description: string;
        readonly authors: string; readonly docsUrl: string; readonly supportUrl: string;
    }
    export interface PackMeta { readonly reference: string; readonly name: string; readonly version: string; readonly priority: number; }
    export interface ConditionMap {
        readonly gameVersion?: string;
        readonly hasMod?: string | readonly string[];
        readonly modVersion?: Readonly<Record<string, string>>;
        readonly hasClass?: string | readonly string[];
        readonly config?: string | readonly string[];
        readonly ifNotMatch?: boolean;
    }
    export interface Conditions {
        all(conditions: readonly boolean[]): boolean;
        any(conditions: readonly boolean[]): boolean;
        evaluate(conditions: ConditionMap | readonly ConditionMap[], behavior?: ConditionBehavior): boolean;
        evaluate(conditions: ConditionMap | readonly ConditionMap[], any: boolean): ConditionResult;
        evaluateResult(conditions: ConditionMap | readonly ConditionMap[], any?: boolean): ConditionResult;
    }
    export type ConfigScalar = boolean | number | bigint | string;
    export type ConfigValue = ConfigScalar | readonly ConfigScalar[];
    export interface ConfigValueMap { }
    export interface ConfigChange<T = ConfigValue> { readonly id: string; readonly value: T; readonly previous: T; }
    export interface PackConfig {
        has(id: string): boolean;
        snapshot(id: string): ConfigSnapshot;
        onChange(id: string, key: string, callback: (snapshot: ConfigSnapshot) => void): Subscription;
        get<K extends keyof ConfigValueMap & string>(id: K): ConfigValueMap[K] | null;
        get(id: string): ConfigValue | null;
        onChange<K extends keyof ConfigValueMap & string>(id: K, callback: (change: ConfigChange<ConfigValueMap[K]>) => void): Subscription;
        onChange(id: string, callback: (change: ConfigChange) => void): Subscription;
    }
    export interface ActorFilter {
        readonly nameContains?: string; readonly pathPrefix?: string;
        readonly tags?: string | readonly string[]; readonly tagMatch?: TagMatch;
        readonly exactClass?: boolean; readonly caseSensitive?: boolean;
    }
    export interface ActorQuery extends ActorFilter { readonly limit?: number; readonly after?: string; }
    export interface WorldActorQuery extends ActorFilter { readonly limit?: number; }
    export interface ActorPage<T extends UObject> { readonly actors: readonly T[]; readonly nextCursor: string | null; }
    export type ActorChange<T extends UObject> = Readonly<{ kind: ActorChangeKind.Added; actor: T; path: string }> | Readonly<{ kind: ActorChangeKind.Removed; actor: null; path: string }>;
    export interface ActorObservationOptions { readonly includeExisting?: boolean; }
    export interface ConditionResult {
        readonly outcome: ConditionOutcome;
        readonly readConfigKeys: readonly string[];
    }
    export interface ConfigSnapshot {
        readonly found: boolean; readonly value: ConfigValue | undefined; readonly revision: bigint;
        readonly cold: boolean; readonly hostMismatch: boolean;
    }
    export type ConfigAccess = PackConfig;
    export interface KDFError extends Error {
        readonly name: "KDFError"; readonly code: string;
        readonly details: { readonly source: string; readonly line: number; readonly column: number };
    }
    export interface RuntimeStats {
        /** Actual native authoring binding wall time; diagnostics only, not deterministic gameplay input. */
        readonly authoringBindingCalls: bigint; readonly authoringBindingSeconds: number;
        readonly suppressedConsoleMessages: bigint; readonly consoleMessages: number; readonly consoleBytes: number;
        readonly maxConsoleMessages: number; readonly maxConsoleBytes: number;
        readonly subscriptions: number; readonly maxSubscriptions: number;
    }
    export interface SeededRandom { next(): number; nextUInt32(): number; }
    export interface WorldQueryResult<T extends UObject> {
        readonly items: readonly T[]; readonly visited: number; readonly truncated: boolean;
    }
    export interface WorldAccess {
        info(): { readonly mapName: string; readonly dedicatedServer: boolean; readonly gameTimeSeconds: number };
        findActors<C extends Class<UObject>>(type: C, query?: number | WorldActorQuery): WorldQueryResult<ClassValue<C>>;
        /** Requires an imported PlayerState-derived class and runtime server authority. */
        players(query?: number | WorldActorQuery): WorldQueryResult<UObject>;
    }
    export type EventSchema = Readonly<Record<string, EventScalarType>>;
    export type EventPayload<S extends EventSchema> = {
        readonly [K in keyof S]: S[K] extends EventScalarType.Boolean ? boolean :
            S[K] extends EventScalarType.Int64 | EventScalarType.UInt64 ? bigint : S[K] extends EventScalarType.Number ? number : string;
    };
    const customEventBrand: unique symbol;
    export interface CustomEvent<S extends EventSchema> { readonly [customEventBrand]: S; }
    export type Comparable = string | number | bigint;
    export type Falsy = false | 0 | 0n | "" | null | undefined;
    export type Truthy<T> = T extends Falsy ? never : T;
    export type OmitKeys<T, K extends keyof T> = Pick<T, Exclude<keyof T, K>>;
    export interface ArrayHelpers {
        range(end: number): number[]; range(start: number, end: number, step?: number): number[];
        sample<T>(values: readonly T[], random?: () => number): T | undefined;
        shuffle<T>(values: readonly T[], random?: () => number): T[];
        windowed<T>(values: readonly T[], size: number, step?: number): T[][];
        chunk<T>(values: readonly T[], size: number): T[][];
        compact<T>(values: readonly T[]): Array<Truthy<T>>;
        unique<T>(values: readonly T[]): T[];
        uniqueBy<T, K>(values: readonly T[], select: (value: T, index: number) => K): T[];
        groupBy<T, K extends PropertyKey>(values: readonly T[], select: (value: T, index: number) => K): Record<K, T[]>;
        keyBy<T, K extends PropertyKey>(values: readonly T[], select: (value: T, index: number) => K): Record<K, T>;
        partition<T>(values: readonly T[], predicate: (value: T, index: number) => unknown): [T[], T[]];
        flatten<T>(values: readonly (T | readonly T[])[]): T[];
        flattenDeep<T>(values: readonly unknown[]): T[];
        first<T>(values: readonly T[]): T | undefined;
        last<T>(values: readonly T[]): T | undefined;
        initial<T>(values: readonly T[]): T[];
        tail<T>(values: readonly T[]): T[];
        take<T>(values: readonly T[], count: number): T[];
        takeRight<T>(values: readonly T[], count: number): T[];
        drop<T>(values: readonly T[], count: number): T[];
        dropRight<T>(values: readonly T[], count: number): T[];
        zip(...values: readonly (readonly unknown[])[]): unknown[][];
        unzip<T>(values: readonly (readonly T[])[]): Array<Array<T | undefined>>;
        rotate<T>(values: readonly T[], amount: number): T[];
        difference<T>(values: readonly T[], excluded: readonly T[]): T[];
        intersection<T>(values: readonly T[], other: readonly T[]): T[];
        union<T>(...values: readonly (readonly T[])[]): T[];
        sortBy<T, K extends Comparable>(values: readonly T[], select: (value: T, index: number) => K): T[];
        minBy<T, K extends Comparable>(values: readonly T[], select: (value: T, index: number) => K): T | undefined;
        maxBy<T, K extends Comparable>(values: readonly T[], select: (value: T, index: number) => K): T | undefined;
        countBy<T, K extends PropertyKey>(values: readonly T[], select: (value: T, index: number) => K): Record<K, number>;
    }
    export interface StringHelpers {
        capitalize(value: string): string; decapitalize(value: string): string; words(value: string): string[];
        camelCase(value: string): string; pascalCase(value: string): string; kebabCase(value: string): string;
        snakeCase(value: string): string; titleCase(value: string): string; slugify(value: string): string;
        truncate(value: string, length: number, suffix?: string): string; padCenter(value: string, length: number, fill?: string): string;
        reverse(value: string): string; countOccurrences(value: string, search: string): number;
        removePrefix(value: string, prefix: string): string; removeSuffix(value: string, suffix: string): string;
        ensurePrefix(value: string, prefix: string): string; ensureSuffix(value: string, suffix: string): string;
        before(value: string, search: string): string; after(value: string, search: string): string;
        between(value: string, start: string, end: string): string; lines(value: string): string[];
        normalizeWhitespace(value: string): string; isBlank(value: string): boolean; escapeRegExp(value: string): string;
        stripIndent(value: string): string;
    }
    export interface NumberHelpers {
        clamp(value: number, minimum: number, maximum: number): number; lerp(start: number, end: number, alpha: number): number;
        inverseLerp(start: number, end: number, value: number): number;
        remap(value: number, sourceStart: number, sourceEnd: number, targetStart: number, targetEnd: number): number;
        wrap(value: number, minimum: number, maximum: number): number; roundTo(value: number, step?: number): number;
        floorTo(value: number, step?: number): number; ceilTo(value: number, step?: number): number;
        sum(values: readonly number[]): number; sumBy<T>(values: readonly T[], select: (value: T, index: number) => number): number;
        average(values: readonly number[]): number | undefined;
        averageBy<T>(values: readonly T[], select: (value: T, index: number) => number): number | undefined;
        median(values: readonly number[]): number | undefined; mode(values: readonly number[]): number | undefined;
        min(values: readonly number[]): number | undefined; max(values: readonly number[]): number | undefined;
        product(values: readonly number[]): number; percentage(value: number, total: number): number;
        percentOf(percent: number, total: number): number; nearlyEqual(left: number, right: number, epsilon?: number): boolean;
        signNonZero(value: number): NumericSign; inRange(value: number, minimum: number, maximum: number, inclusive?: boolean): boolean;
    }
    export interface ObjectHelpers {
        pick<T extends object, K extends keyof T>(value: T, keys: readonly K[]): Pick<T, K>;
        omit<T extends object, K extends keyof T>(value: T, keys: readonly K[]): OmitKeys<T, K>;
        mapValues<T extends object, R>(value: T, transform: (item: T[keyof T], key: string) => R): Record<string, R>;
        mapKeys<T extends object, K extends PropertyKey>(value: T, transform: (key: string, item: T[keyof T]) => K): Record<K, T[keyof T]>;
        filterEntries<T extends object>(value: T, predicate: (item: T[keyof T], key: string) => unknown): Partial<T>;
        invert(value: Readonly<Record<string, PropertyKey>>): Record<PropertyKey, string>;
        fromPairs<K extends PropertyKey, V>(pairs: Iterable<readonly [K, V]>): Record<K, V>;
        toPairs<T extends object>(value: T): Array<[string, T[keyof T]]>;
        mergeDefined<T extends object>(...values: readonly Partial<T>[]): Partial<T>;
        getPath<T = unknown>(value: unknown, path: readonly PropertyKey[], fallback?: T): T | undefined;
        hasPath(value: unknown, path: readonly PropertyKey[]): boolean;
        setPath<T>(value: T, path: readonly PropertyKey[], nextValue: unknown): T;
        deepFreeze<T>(value: T): Readonly<T>; isEmpty(value: object): boolean;
        entriesSorted<T extends object>(value: T): Array<[string, T[keyof T]]>;
    }
    export interface GuardHelpers {
        isDefined<T>(value: T | undefined): value is T; isNil(value: unknown): value is null | undefined;
        isBoolean(value: unknown): value is boolean; isNumber(value: unknown): value is number;
        isFiniteNumber(value: unknown): value is number; isInteger(value: unknown): value is number;
        isSafeInteger(value: unknown): value is number; isBigInt(value: unknown): value is bigint;
        isString(value: unknown): value is string; isNonEmptyString(value: unknown): value is string;
        isArray(value: unknown): value is unknown[]; isObject(value: unknown): value is object;
        isPlainObject(value: unknown): value is Record<PropertyKey, unknown>; isFunction(value: unknown): value is Function;
        isPromiseLike<T = unknown>(value: unknown): value is PromiseLike<T>; isIterable<T = unknown>(value: unknown): value is Iterable<T>;
        isMap(value: unknown): value is Map<unknown, unknown>; isSet(value: unknown): value is Set<unknown>;
        assertDefined<T>(value: T | null | undefined, message?: string): T; assertNever(value: never, message?: string): never;
    }
    export interface CollectionHelpers {
        getOrCreate<K, V>(map: Map<K, V>, key: K, create: (key: K) => V): V;
        mapMap<K, V, RK, RV>(map: Map<K, V>, transform: (value: V, key: K) => readonly [RK, RV]): Map<RK, RV>;
        filterMap<K, V>(map: Map<K, V>, predicate: (value: V, key: K) => unknown): Map<K, V>;
        mapSet<T, R>(set: Set<T>, transform: (value: T) => R): Set<R>;
        filterSet<T>(set: Set<T>, predicate: (value: T) => unknown): Set<T>;
        toggleSet<T>(set: Set<T>, value: T, force?: boolean): boolean;
        addToMapSet<K, V>(map: Map<K, Set<V>>, key: K, value: V): Set<V>;
        removeFromMapSet<K, V>(map: Map<K, Set<V>>, key: K, value: V): boolean;
        frequency<T>(values: Iterable<T>): Map<T, number>; hasDuplicates<T>(values: Iterable<T>): boolean;
    }
    export interface FunctionHelpers {
        identity<T>(value: T): T; constant<T>(value: T): () => T; noop(): void;
        tap<T>(value: T, effect: (value: T) => void): T; negate<A extends readonly unknown[]>(predicate: (...args: A) => unknown): (...args: A) => boolean;
        once<A extends readonly unknown[], R>(callback: (...args: A) => R): (...args: A) => R;
        memoize<A extends readonly unknown[], K, R>(callback: (...args: A) => R, selectKey?: (...args: A) => K): (...args: A) => R;
        pipe<A, B>(first: (value: A) => B): (value: A) => B;
        pipe<A, B, C>(first: (value: A) => B, second: (value: B) => C): (value: A) => C;
        pipe<A, B, C, D>(first: (value: A) => B, second: (value: B) => C, third: (value: C) => D): (value: A) => D;
        compose<A, B>(first: (value: A) => B): (value: A) => B;
        compose<A, B, C>(second: (value: B) => C, first: (value: A) => B): (value: A) => C;
        compose<A, B, C, D>(third: (value: C) => D, second: (value: B) => C, first: (value: A) => B): (value: A) => D;
        times<T>(count: number, callback: (index: number) => T): T[];
        tryOr<T>(callback: () => T, fallback: T | ((error: unknown) => T)): T;
    }
    export type Settled<T> = Readonly<{ status: AsyncResultStatus.Fulfilled; value: T }> | Readonly<{ status: AsyncResultStatus.Rejected; reason: unknown }>;
    export interface PromiseHelpers {
        mapSeries<T, R>(values: readonly T[], transform: (value: T, index: number) => R | PromiseLike<R>): Promise<R[]>;
        filterSeries<T>(values: readonly T[], predicate: (value: T, index: number) => unknown | PromiseLike<unknown>): Promise<T[]>;
        eachSeries<T>(values: readonly T[], effect: (value: T, index: number) => unknown | PromiseLike<unknown>): Promise<void>;
        settle<T>(value: T | PromiseLike<T>): Promise<Settled<T>>;
    }
    export interface SemVer {
        readonly major: number; readonly minor: number; readonly patch: number;
        readonly prerelease: readonly (string | number)[]; readonly build: readonly string[];
    }
    export type SemVerInput = string | SemVer;
    /** Semantic Versioning 2.0 with npm-style ranges: `^`, `~`, `>=`, `1.x`, `1.2 - 1.4`, `||`, space for AND. */
    export interface SemVerHelpers {
        parse(value: string): SemVer | null;
        create(major: number, minor?: number, patch?: number, prerelease?: readonly (string | number)[], build?: readonly string[]): SemVer;
        format(version: SemVerInput): string; isValid(value: unknown): boolean; coerce(value: unknown): SemVer | null;
        compare(left: SemVerInput, right: SemVerInput): Ordering;
        eq(left: SemVerInput, right: SemVerInput): boolean; neq(left: SemVerInput, right: SemVerInput): boolean;
        gt(left: SemVerInput, right: SemVerInput): boolean; gte(left: SemVerInput, right: SemVerInput): boolean;
        lt(left: SemVerInput, right: SemVerInput): boolean; lte(left: SemVerInput, right: SemVerInput): boolean;
        satisfies(version: SemVerInput, range: string): boolean;
        maxSatisfying<T extends SemVerInput>(versions: readonly T[], range: string): T | undefined;
        sort<T extends SemVerInput>(versions: readonly T[]): T[];
        inc(version: SemVerInput, release: SemVerRelease): SemVer;
    }
    export const arrays: Readonly<ArrayHelpers>;
    export const strings: Readonly<StringHelpers>;
    export const numbers: Readonly<NumberHelpers>;
    export const objects: Readonly<ObjectHelpers>;
    export const guards: Readonly<GuardHelpers>;
    export const collections: Readonly<CollectionHelpers>;
    export const functions: Readonly<FunctionHelpers>;
    export const promises: Readonly<PromiseHelpers>;
    export const semver: Readonly<SemVerHelpers>;
    /** Canonical descriptor identity. `id` names authored content; `classPath` names an existing soft class path. */
    export type ItemReference = Readonly<{ id: string }> | Readonly<{ classPath: string }>;
    export interface LocalizedText { readonly namespace: string; readonly key: string; readonly source: string; }
    export interface ItemAmount { readonly item: ItemReference; readonly amount: number | bigint; }
    /** Members of an exported class that a definition can write: the scalar, enum and string properties of its descriptor. */
    export type WritableKeys<T> = { [K in keyof T]-?: T[K] extends WritableValueRef<any> ? K : never }[keyof T];
    export type PropertyWrites<T> = { readonly [K in WritableKeys<T>]?: T[K] extends WritableValueRef<infer V> ? V : never };
    /** Values for properties the export does not describe: class references (handles), lists and nested structs. */
    export type ExtraValue = boolean | number | bigint | string | Class<UObject> | readonly ExtraValue[] | { readonly [key: string]: ExtraValue };
    export interface ExtraWrites { readonly [property: string]: ExtraValue }
    export interface ItemDefinition<C extends Class<UObject> = Class<UObject>> {
        /** Pass the exported parent class (for example `FGConsumableDescriptor`) to type `properties` from its members. */
        readonly id: string; readonly parent: ItemReference | C; readonly displayName: LocalizedText;
        readonly form: ResourceForm;
        readonly stackSize: StackSize;
        readonly description?: LocalizedText; readonly energyValueMJ?: number; readonly radioactiveDecay?: number;
        readonly canBeDiscarded?: boolean; readonly categories?: readonly string[];
        /** Typed writes on the generated class, limited to what the parent's exported descriptor exposes. */
        readonly properties?: PropertyWrites<ClassValue<C>>;
        /** Anything the export does not describe, such as sub categories as class handles. */
        readonly extra?: ExtraWrites;
    }
    export interface RecipeDefinition<C extends Class<UObject> = Class<UObject>> {
        /** Defaults to FGRecipe. Non-default parent must be an authorized, loaded recipe class. */
        readonly parent?: ItemReference | C;
        readonly id: string; readonly ingredients: readonly ItemAmount[]; readonly products: readonly ItemAmount[];
        readonly producedIn: readonly ItemReference[]; readonly durationSeconds: number;
        readonly displayName?: LocalizedText; readonly manualMultiplier?: number; readonly menuPriority?: number | bigint;
        readonly register?: boolean; readonly categories?: readonly string[];
        readonly properties?: PropertyWrites<ClassValue<C>>;
        /** Property writes on the generated recipe class that the fixed fields do not cover. */
        readonly extra?: ExtraWrites;
    }
    export interface ContentRef<K extends ContentKind> { readonly kind: K; readonly id: string; }
    /** Generic bounded document shape used by catalog tooling. Authors should prefer HandlerDocument<K>. */
    export interface KDFDocument extends StructuredInputObject { readonly type: HandlerKey; }
    /** One bounded handler-owned map entry. Handler validation still enforces its detailed native schema. */
    export type KDFDocumentEntry = Readonly<Record<string, StructuredInputValue>>;
    export interface KDFPropertyOperation extends KDFDocumentEntry { readonly path: string; readonly op?: PropertyOperation; readonly value?: StructuredInputValue; }
    /** Shared create/update schema used by the native content-class handlers. Property paths stay handler-validated. */
    export interface GeneratedContentEntry extends KDFDocumentEntry {
        readonly id: string; readonly parent?: string; readonly properties?: readonly KDFPropertyOperation[];
        readonly register?: boolean;
    }
    export interface RegisteredContentEntry extends KDFDocumentEntry {
        readonly class: string; readonly properties?: readonly KDFPropertyOperation[]; readonly register?: boolean;
    }
    export type ContentClassEntry = GeneratedContentEntry | RegisteredContentEntry;
    /** Native removal filters are validated by the selected handler and use the same transactional document seam. */
    export type ContentRemoval = string | KDFDocumentEntry;
    export interface KDFCdoPatch extends KDFDocumentEntry { readonly target?: string; readonly properties: readonly KDFPropertyOperation[]; }
    export interface CdoDocument extends KDFDocument { readonly type: DocumentType.Cdo; readonly patches: readonly KDFCdoPatch[]; }
    export interface AssetDocument extends KDFDocument { readonly type: DocumentType.Asset; readonly assets: readonly KDFDocumentEntry[]; }
    export interface DataAssetDocument extends KDFDocument { readonly type: DocumentType.DataAsset; readonly assets: readonly KDFDocumentEntry[]; }
    export interface ItemDocument extends KDFDocument { readonly type: DocumentType.Item; readonly items: readonly GeneratedContentEntry[]; }
    export interface ResourceDocument extends KDFDocument { readonly type: DocumentType.Resource; readonly resources: readonly GeneratedContentEntry[]; }
    export interface BuildingDocument extends KDFDocument { readonly type: DocumentType.Building; readonly buildings: readonly GeneratedContentEntry[]; }
    export interface UnlockDocument extends KDFDocument { readonly type: DocumentType.Unlock; readonly unlocks: readonly GeneratedContentEntry[]; }
    export interface RecipeDocument extends KDFDocument { readonly type: DocumentType.Recipe; readonly recipes?: readonly ContentClassEntry[]; readonly remove?: readonly ContentRemoval[]; }
    export interface SchematicDocument extends KDFDocument { readonly type: DocumentType.Schematic; readonly schematics?: readonly ContentClassEntry[]; readonly remove?: readonly ContentRemoval[]; }
    export interface ResearchDocument extends KDFDocument { readonly type: DocumentType.Research; readonly research?: readonly ContentClassEntry[]; readonly remove?: readonly ContentRemoval[]; }
    export interface CurveDocument extends KDFDocument { readonly type: DocumentType.Curve; readonly curveType: CurveType; readonly curves: readonly KDFDocumentEntry[]; }
    export interface LocalizationDocument extends KDFDocument { readonly type: DocumentType.Localization; readonly namespace: string; readonly culture?: string; readonly entries: Readonly<Record<string, StructuredInputValue>>; }
    export interface GameTagDocument extends KDFDocument { readonly type: DocumentType.GameTag; readonly tags: readonly StructuredInputValue[]; }
    export interface SinkPointsDocument extends KDFDocument { readonly type: DocumentType.SinkPoints; readonly entries: readonly KDFDocumentEntry[]; readonly track?: SinkTrack; }
    export interface ResourceNodeDocument extends KDFDocument { readonly type: DocumentType.ResourceNode; }
    export interface SoundDocument extends KDFDocument { readonly type: DocumentType.Sound; readonly sounds: readonly KDFDocumentEntry[]; }
    export interface SublevelDocument extends KDFDocument { readonly type: DocumentType.Sublevel; readonly block: readonly StructuredInputValue[]; }
    export interface EventRuleDocument extends KDFDocument { readonly type: DocumentType.EventRule; readonly rules: readonly KDFDocumentEntry[]; }
    export interface CategoryReorderDocument extends KDFDocument { readonly type: DocumentType.CategoryReorder; readonly subType: CategoryKind; readonly category: string; readonly order: readonly string[]; readonly subCategory?: string; readonly categoryPriority?: number; readonly subCategoryPriority?: number; }
    export interface ClassEntryDefinition extends StructuredInputObject {
        readonly id: string; readonly parent: string; readonly properties?: readonly StructuredInputValue[];
        readonly registerAs?: DocumentType.Recipe | DocumentType.Schematic | DocumentType.Research;
    }
    /** DynamicContent class document. Script methods are supplied separately through defineBehavior. */
    export interface ClassDefinition extends KDFDocument { readonly type: DocumentType.Class; readonly classes: readonly [ClassEntryDefinition]; }
    export interface DynamicClassHandle { readonly id: string; }
    export interface HandlerDocumentMap {
        readonly [DocumentType.Asset]: AssetDocument; readonly [DocumentType.CategoryReorder]: CategoryReorderDocument;
        readonly [DocumentType.Cdo]: CdoDocument; readonly [DocumentType.Class]: ClassDefinition; readonly [DocumentType.Curve]: CurveDocument;
        readonly [DocumentType.DataAsset]: DataAssetDocument; readonly [DocumentType.EventRule]: EventRuleDocument; readonly [DocumentType.GameTag]: GameTagDocument;
        readonly [DocumentType.Localization]: LocalizationDocument; readonly [DocumentType.Item]: ItemDocument; readonly [DocumentType.Resource]: ResourceDocument;
        readonly [DocumentType.Building]: BuildingDocument; readonly [DocumentType.Unlock]: UnlockDocument; readonly [DocumentType.Recipe]: RecipeDocument;
        readonly [DocumentType.Schematic]: SchematicDocument; readonly [DocumentType.Research]: ResearchDocument;
        readonly [DocumentType.ResourceNode]: ResourceNodeDocument; readonly [DocumentType.SinkPoints]: SinkPointsDocument;
        readonly [DocumentType.Sound]: SoundDocument; readonly [DocumentType.Sublevel]: SublevelDocument;
    }
    export type HandlerKey = keyof HandlerDocumentMap & string;
    export type HandlerDocument<K extends HandlerKey> = HandlerDocumentMap[K];

    /** Frozen context supplied to default main. Every member below is installed by KDFJSRuntime. */
    export type Promisify<F> = F extends (...args: infer A) => infer R ? (...args: A) => Promise<Awaited<R>> : never;
    export interface PromiseMethodMap { readonly catalog: true; readonly runtimeStats: true; readonly lookupItem: true; readonly lookupRecipe: true; readonly resolve: true; readonly removeItem: true; readonly removeRecipe: true; readonly defineClass: true; readonly import: true; readonly listMods: true; readonly getModMeta: true; readonly isModLoaded: true; readonly listPacks: true; readonly getPackMeta: true; readonly isPackLoaded: true; readonly gameVersion: true; readonly spawnActorById: true; readonly destroyActor: true; readonly modifyActorById: true; readonly createBehavior: true; readonly destroyBehavior: true; readonly unregisterBehavior: true; readonly unregisterBehaviorClass: true; }
    export type PromiseRootMethod = keyof PromiseMethodMap;
    /** Settled wrappers around immediate host calls. Calls still execute synchronously; rejection captures thrown KDF errors. */
    export type PromiseContext = Readonly<{ [K in PromiseRootMethod]: Promisify<Context[K]> }> & {
        readonly getClass: {
            <C extends Class<UObject>, P extends NativeClassPathFor<C>>(type: C, path: P, options: CheckedLookupOptions): Promise<C>;
            <C extends Class<UObject>, P extends NativeClassPathFor<C>>(type: C, path: P, options?: LookupOptions): Promise<C | null>;
        };
        readonly getObject: {
            <C extends Class<UObject>>(type: C, path: string, options: CheckedLookupOptions): Promise<ClassValue<C>>;
            <C extends Class<UObject>>(type: C, path: string, options?: LookupOptions): Promise<ClassValue<C> | null>;
        };
        readonly getDataAsset: {
            <C extends Class<UObject>>(type: C, path: string, options: CheckedLookupOptions): Promise<ClassValue<C>>;
            <C extends Class<UObject>>(type: C, path: string, options?: LookupOptions): Promise<ClassValue<C> | null>;
        };
        readonly cast: {
            <C extends Class<UObject>>(type: C, value: Class<UObject>, options: CheckedLookupOptions): Promise<Class<ClassValue<C>>>;
            <C extends Class<UObject>>(type: C, value: Class<UObject>, options?: LookupOptions): Promise<Class<ClassValue<C>> | null>;
            <C extends Class<UObject>>(type: C, value: UObject, options: CheckedLookupOptions): Promise<ClassValue<C>>;
            <C extends Class<UObject>>(type: C, value: UObject, options?: LookupOptions): Promise<ClassValue<C> | null>;
        };
        readonly findClasses: <C extends Class<UObject>>(type: C, search?: ClassSearchParams) => Promise<readonly C[]>;
        readonly findActors: <C extends Class<UObject>>(type: C, query?: ActorQuery) => Promise<ActorPage<ClassValue<C>>>;
        readonly batch: <T extends readonly InputValue[]>(writes: {
            readonly [K in keyof T]: { readonly property: WritableValueRef<T[K]>; readonly value: NoInfer<T[K]> };
        }) => Promise<void>;
        readonly createItem: <C extends Class<UObject> = Class<UObject>>(definition: ItemDefinition<C>) => Promise<ContentRef<ContentKind.Item>>;
        readonly updateItem: <C extends Class<UObject> = Class<UObject>>(definition: ItemDefinition<C>) => Promise<ContentRef<ContentKind.Item>>;
        readonly createRecipe: <C extends Class<UObject> = Class<UObject>>(definition: RecipeDefinition<C>) => Promise<ContentRef<ContentKind.Recipe>>;
        readonly updateRecipe: <C extends Class<UObject> = Class<UObject>>(definition: RecipeDefinition<C>) => Promise<ContentRef<ContentKind.Recipe>>;
        readonly applyDocument: <K extends HandlerKey>(handlerId: K, document: HandlerDocumentMap[NoInfer<K>]) => Promise<void>;
        readonly spawnActor: <K extends ActorClassKey>(
            classDescriptorId: K, initialWrites?: ActorInitialWrites<K>
        ) => Promise<CatalogActor<K>>;
        readonly modifyActor: <K extends ActorClassKey, P extends ActorPropertyKey<K>>(
            actor: CatalogActor<K>, propertyId: P, value: ActorPropertyValue<K, P>
        ) => Promise<void>;
        readonly config: Readonly<{
            has(id: string): Promise<boolean>; snapshot(id: string): Promise<ConfigSnapshot>;
            get<K extends keyof ConfigValueMap & string>(id: K): Promise<ConfigValueMap[K] | null>;
            get(id: string): Promise<ConfigValue | null>;
        }>;
        readonly world: Readonly<{
            info(): Promise<{ readonly mapName: string; readonly dedicatedServer: boolean; readonly gameTimeSeconds: number }>;
            findActors<C extends Class<UObject>>(type: C, query?: number | WorldActorQuery): Promise<WorldQueryResult<ClassValue<C>>>;
            players(query?: number | WorldActorQuery): Promise<WorldQueryResult<UObject>>;
        }>;
        readonly conditions: Readonly<{
            evaluate(conditions: ConditionMap | readonly ConditionMap[], behavior?: ConditionBehavior): Promise<boolean>;
            evaluate(conditions: ConditionMap | readonly ConditionMap[], any: boolean): Promise<ConditionResult>;
            evaluateResult(conditions: ConditionMap | readonly ConditionMap[], any?: boolean): Promise<ConditionResult>;
        }>;
        readonly diagnostics: Readonly<{
            add(severity: DiagnosticSeverity, message: string, authoredPath?: string): Promise<void>;
        }>;
    };

    export interface Context {
        readonly catalog: () => Catalog;
        /** Bounded, side-effect-free summary: no getters, proxies, toJSON or object coercion. */
        readonly inspect: (value: unknown) => string;
        readonly runtimeStats: () => RuntimeStats;
        readonly diagnostics: { add(severity: DiagnosticSeverity, message: string, authoredPath?: string): void; };
        readonly world: WorldAccess;
        readonly promises: PromiseContext;
        /** Local game time, affected by pause/dilation; never a cross-peer deterministic clock. */
        readonly now: () => number;
        readonly random: (seed: string) => SeededRandom;
        readonly defineEvent: <const S extends EventSchema>(id: string, schema: S) => CustomEvent<S>;
        readonly onCustom: <S extends EventSchema>(event: CustomEvent<S>, key: string, callback: (payload: EventPayload<NoInfer<S>>) => void, options?: EventOptions<EventPayload<NoInfer<S>>>) => Subscription;
        readonly emit: <S extends EventSchema>(event: CustomEvent<S>, payload: EventPayload<NoInfer<S>>) => void;
        /** Atomic, bounded property transaction. Tracked refs and function calls are rejected. */
        readonly batch: <T extends readonly unknown[]>(writes: {
            readonly [K in keyof T]: { readonly property: WritableValueRef<T[K]>; readonly value: NoInfer<T[K]> };
        }) => void;
        readonly lookupItem: (id: string, pack?: string) => Class<UObject> | null;
        readonly lookupRecipe: (id: string, pack?: string) => Class<UObject> | null;
        readonly resolve: (reference: ContentRef<ContentKind>, pack?: string) => Class<UObject> | null;
        /** Lowercase MD5 digest of UTF-8 text. */
        readonly md5: (text: string) => string;
        /** Lowercase SHA-1 digest of UTF-8 text. */
        readonly sha1: (text: string) => string;
        /** Lowercase SHA-256 digest of UTF-8 text. */
        readonly sha256: (text: string) => string;
        /** Eight-character lowercase CRC-32 digest of UTF-8 text. */
        readonly crc32: (text: string) => string;
        /** Random UUID in lowercase RFC 4122 hyphenated form. */
        readonly uuid: () => string;
        readonly createCleanupBag: () => CleanupBag;
        readonly isValid: (value: Target | null | undefined) => boolean;
        readonly target: { (): Realm; (name: "realm"): Realm; };
        readonly getClass: {
            <C extends Class<UObject>, P extends NativeClassPathFor<C>>(type: C, path: P, options: CheckedLookupOptions): C;
            <C extends Class<UObject>, P extends NativeClassPathFor<C>>(type: C, path: P, options?: LookupOptions): C | null;
        };
        readonly getObject: {
            <C extends Class<UObject>>(type: C, path: string, options: CheckedLookupOptions): ClassValue<C>;
            <C extends Class<UObject>>(type: C, path: string, options?: LookupOptions): ClassValue<C> | null;
        };
        readonly getDataAsset: {
            <C extends Class<UObject>>(type: C, path: string, options: CheckedLookupOptions): ClassValue<C>;
            <C extends Class<UObject>>(type: C, path: string, options?: LookupOptions): ClassValue<C> | null;
        };
        readonly cast: {
            <C extends Class<UObject>>(type: C, value: Class<UObject>, options: CheckedLookupOptions): Class<ClassValue<C>>;
            <C extends Class<UObject>>(type: C, value: Class<UObject>, options?: LookupOptions): Class<ClassValue<C>> | null;
            <C extends Class<UObject>>(type: C, value: UObject, options: CheckedLookupOptions): ClassValue<C>;
            <C extends Class<UObject>>(type: C, value: UObject, options?: LookupOptions): ClassValue<C> | null;
        };
        readonly findClasses: <C extends Class<UObject>>(type: C, search?: ClassSearchParams) => readonly C[];
        readonly config: PackConfig;
        readonly findActors: <C extends Class<UObject>>(type: C, query?: ActorQuery) => ActorPage<ClassValue<C>>;
        readonly observeActors: <C extends Class<UObject>>(type: C, callback: (change: ActorChange<ClassValue<C>>) => void, options?: ActorObservationOptions) => Subscription;
        readonly createItem: <C extends Class<UObject> = Class<UObject>>(definition: ItemDefinition<C>) => ContentRef<ContentKind.Item>;
        readonly updateItem: <C extends Class<UObject> = Class<UObject>>(definition: ItemDefinition<C>) => ContentRef<ContentKind.Item>;
        readonly removeItem: (id: string) => ContentRef<ContentKind.Item>;
        readonly createRecipe: <C extends Class<UObject> = Class<UObject>>(definition: RecipeDefinition<C>) => ContentRef<ContentKind.Recipe>;
        readonly updateRecipe: <C extends Class<UObject> = Class<UObject>>(definition: RecipeDefinition<C>) => ContentRef<ContentKind.Recipe>;
        readonly removeRecipe: (id: string) => ContentRef<ContentKind.Recipe>;
        readonly applyDocument: <K extends HandlerKey>(handlerId: K, document: HandlerDocumentMap[NoInfer<K>]) => void;
        readonly defineClass: (definition: ClassDefinition) => DynamicClassHandle;
        readonly import: (kind: ImportKind, path: string) => StructuredValue;
        readonly compare: (left: InputValue, operator: CompareOperator, right: InputValue) => boolean;
        readonly isAuthority: () => boolean;
        /** True when SML loaded the mod. With `range` (same syntax as the `modVersion` condition, e.g. `^3.12.0`) its version must match too. */
        readonly isModLoaded: (reference: string, range?: string) => boolean;
        /** Metadata of a loaded mod, or `null` when it is not loaded. */
        readonly getModMeta: (reference: string) => ModMeta | null;
        readonly listMods: () => readonly ModMeta[];
        /** True when the KDataForge pack takes part in the current load. With `range` its pack.yml version must match too. */
        readonly isPackLoaded: (reference: string, range?: string) => boolean;
        readonly getPackMeta: (reference: string) => PackMeta | null;
        readonly listPacks: () => readonly PackMeta[];
        /** Game build number, the value `gameVersion:` conditions compare. */
        readonly gameVersion: () => number;
        readonly conditions: Conditions;
        readonly on: {
            (target: Target, eventId: string, callback: PositionalEventCallback): Subscription;
            (target: Target, eventId: string, callback: NamedEventCallback): Subscription;
            <K extends string>(target: Target, eventId: string, key: K, callback: PositionalEventCallback, options?: EventOptions): Subscription;
            <K extends string>(target: Target, eventId: string, key: K, callback: NamedEventCallback, options?: EventOptions): Subscription;
            <K extends string, TKey extends string>(eventId: K, key: TKey, callback: PositionalEventCallback | NamedEventCallback, options?: EventOptions): Subscription;
        };
        readonly off: (subscription: Subscription) => void;
        readonly unregisterEvent: (key: string) => void;
        readonly hook: {
            <T extends UObject, A extends readonly Value[], R>(target: Class<T>, hook: Hook<T, A, R>, callback: (call: HookCall<T, A, R>) => void, options?: HookOptions<A>): Subscription;
            <T extends UObject, A extends readonly Value[], R>(target: Class<T>, hook: Hook<T, A, R>, phase: HookPhase.Before | HookPhase.After, callback: (call: HookCall<T, A, R>) => void, options?: HookOptions<A>): Subscription;
            <T extends UObject, A extends readonly Value[], R>(target: Class<T>, hook: Hook<T, A, R>, phase: HookPhase.Replace, callback: (call: HookCall<T, A, R>) => R | void): Subscription;
        };
        readonly unhook: (subscription: Subscription) => void;
        readonly onTyped: {
            (target: Target, eventId: string, callback: PositionalEventCallback): Subscription;
            (target: Target, eventId: string, callback: NamedEventCallback): Subscription;
            <K extends string>(target: Target, eventId: string, key: K, callback: PositionalEventCallback, options?: EventOptions): Subscription;
            <K extends string>(target: Target, eventId: string, key: K, callback: NamedEventCallback, options?: EventOptions): Subscription;
            <K extends string, TKey extends string>(eventId: K, key: TKey, callback: PositionalEventCallback | NamedEventCallback, options?: EventOptions): Subscription;
        };
        readonly spawnActor: <K extends ActorClassKey>(classDescriptorId: K, initialWrites?: ActorInitialWrites<K>) => CatalogActor<K>;
        /** Explicit untyped escape hatch for tools operating on runtime-discovered catalogs. */
        readonly spawnActorById: (classDescriptorId: string, initialWrites?: Readonly<Record<string, StructuredInputValue>>) => Actor;
        readonly destroyActor: (actor: Actor) => void;
        readonly modifyActor: <K extends ActorClassKey, P extends ActorPropertyKey<K>>(actor: CatalogActor<K>, propertyId: P, value: ActorPropertyValue<K, P>) => void;
        /** Explicit untyped escape hatch for tools operating on runtime-discovered catalogs. */
        readonly modifyActorById: (actor: Actor, propertyId: string, value: StructuredInputValue) => void;
        readonly registerBehavior: (descriptor: BehaviorDescriptor, handlers: Readonly<Record<string, BehaviorHandler>>) => BehaviorRegistration;
        readonly createBehavior: (registration: BehaviorRegistration, target: Actor) => BehaviorInstance;
        readonly destroyBehavior: (instance: BehaviorInstance) => void;
        readonly unregisterBehavior: (instance: BehaviorInstance) => void;
        readonly unregisterBehaviorClass: (registration: BehaviorRegistration, options?: { cascade?: boolean }) => void;
        readonly registerTick: (key: string, callback: (deltaSeconds: number) => void, options: { every: number; requireGameThread?: boolean }) => TickSubscription;
        readonly unregisterTick: (key: string) => void;
        readonly setTimeout: (callback: () => void, seconds: number) => Subscription;
        readonly clearTimeout: (handle: Subscription) => void;
        readonly setInterval: (callback: () => void, seconds: number) => Subscription;
        readonly clearInterval: (handle: Subscription) => void;
        readonly log: (...data: readonly unknown[]) => void; readonly info: (...data: readonly unknown[]) => void;
        readonly warn: (...data: readonly unknown[]) => void; readonly error: (...data: readonly unknown[]) => void;
        readonly debug: (...data: readonly unknown[]) => void;
    }

    export interface CleanupBag {
        readonly add: (cleanup: () => void) => void;
        readonly dispose: () => void;
    }
}

/** ABI 13 / declaration format 10 authoring extensions. Native limits and phase checks apply at every use. */
declare module "@sf-kmods/kdataforgejs" {
    export type Delivery = { readonly mode: DeliveryMode.Immediate } | { readonly mode: DeliveryMode.Debounce | DeliveryMode.Throttle; readonly seconds: number };
    export interface EventOptions<P = Readonly<Record<string, Value>>> {
        readonly priority?: number;
        readonly once?: boolean;
        readonly filter?: Partial<P>;
        readonly delivery?: Delivery;
    }
    export type HookFilter<A extends readonly Value[]> = { readonly [I in Extract<keyof A, `${number}`>]?: A[I] };
    export interface HookOptions<A extends readonly Value[]> { readonly priority?: number; readonly once?: boolean; readonly filter?: HookFilter<A>; }
    export interface HookObservation<A extends readonly Value[], R> { readonly args: A; readonly returnValue?: R; }
    export interface SpatialQuery {
        readonly limit?: number;
        readonly tags?: readonly string[];
        readonly sphere?: { readonly center: readonly [number, number, number]; readonly radius: number };
        readonly box?: { readonly min: readonly [number, number, number]; readonly max: readonly [number, number, number] };
    }
    /** A next/close call invalidates this cursor's preceding page handles. Eight cursors, 30-second idle expiry. */
    export interface ActorCursor<T extends UObject> {
        next(): { readonly items: readonly T[]; readonly visited: number; readonly done: boolean };
        close(): void;
    }
    export interface WorldAccess {
        findActors<C extends Class<UObject>>(type: C, query: SpatialQuery): WorldQueryResult<ClassValue<C>>;
        openQuery<C extends Class<UObject>>(type: C, query?: SpatialQuery): ActorCursor<ClassValue<C>>;
    }
    export interface ContentGraphResult<T extends UObject> extends WorldQueryResult<T> { readonly coverage: "loadedAuthorized"; }
    export interface ContentGraph {
        recipes<C extends Class<UObject>>(type: C, filter?: { readonly ingredient?: Class<UObject>; readonly product?: Class<UObject>; readonly producedIn?: Class<UObject> }, limit?: number): ContentGraphResult<ClassValue<C>>;
        schematicsUnlocking<C extends Class<UObject>>(type: C, recipe: Class<UObject>, limit?: number): ContentGraphResult<ClassValue<C>>;
    }
    const rowSchemaBrand: unique symbol;
    export interface StructType<T> { readonly [rowSchemaBrand]: (value: T) => T; }
    export interface DataTable<T> { get(rowName: string): T | null; set(rowName: string, value: NoInfer<T>): void; }
    const valuePropertiesBrand: unique symbol;
    export interface ValuePropertyTag<T> { readonly [valuePropertiesBrand]: T; }
    export type ValueProperties<T> = T extends ValuePropertyTag<infer P> ? P : { readonly [K in keyof T as T[K] extends ReadonlyValueRef<unknown> ? K : never]: T[K] };
    /** Explicit initialization-only scalar CDO sets. No selectors, insertion, generation or registration. */
    export interface ReversibleCdoDocument {
        readonly type: DocumentType.Cdo;
        readonly patches: readonly { readonly target: string; readonly properties: readonly { readonly path: string; readonly op?: PropertyOperation.Set; readonly value: Exclude<InputValue, null | undefined> }[] }[];
    }
    export interface LifecyclePayloads {
        readonly [LifecycleEvent.WorldReady]: { readonly worldGeneration: bigint };
        readonly [LifecycleEvent.Travel]: { readonly worldGeneration: bigint };
        readonly [LifecycleEvent.Shutdown]: { readonly worldGeneration: bigint; readonly reason: "shutdown" };
    }
    export interface TemporaryOverride extends Subscription { restore(): void; }
    export interface Context {
        readonly hookOnce: {
            <T extends UObject, A extends readonly Value[], R>(target: Class<T>, hook: Hook<T,A,R>, callback: (call: HookCall<T,A,R>) => void, options?: Omit<HookOptions<A>,"once">): Subscription;
            <T extends UObject, A extends readonly Value[], R>(target: Class<T>, hook: Hook<T,A,R>, phase: HookPhase.Before | HookPhase.After, callback: (call: HookCall<T,A,R>) => void, options?: Omit<HookOptions<A>,"once">): Subscription;
        };
        readonly observeHook: <T extends UObject, A extends readonly Value[], R>(target: Class<T>, hook: Hook<T,A,R>, key: string, callback: (call: HookObservation<A,R>) => void, options?: HookOptions<A> & { readonly delivery?: Delivery }) => Subscription;
        readonly lifecycle: { on<K extends keyof LifecyclePayloads>(event: K, key: string, callback: (event: LifecyclePayloads[K]) => void): Subscription };
        readonly applyDocuments: (documents: readonly ReversibleCdoDocument[]) => void;
        readonly dataTables: { open<T>(table: UObject, rowType: StructType<T>): DataTable<T> };
        readonly values: <T extends UObject>(object: T) => ValueProperties<T>;
        readonly graph: ContentGraph;
        /** Scalar actor properties only; 0.001..3600 seconds. Later KDF writes supersede restoration. */
        readonly overrideFor: <T extends Value>(property: WritableValueRef<T>, value: NoInfer<T>, seconds: number) => TemporaryOverride;
    }
}
/** Bounded YAML/JSON data; both codecs preserve quoted strings and decode unquoted booleans/numbers. */
type KDFFileData = null | boolean | number | string | readonly KDFFileData[] | { readonly [key: string]: KDFFileData };
declare const YML: Readonly<{ parse(text: string): KDFFileData; stringify(value: KDFFileData): string }>;
/** Server runtime only. Compiler requires direct static string literal. Reads sandbox override, otherwise immutable attested pack input. */
declare class YMLFile {
    constructor(path: string);
    json(): Promise<KDFFileData>;
    /** Server runtime only, requires kdf.files.write. */
    save(value: KDFFileData): Promise<void>;
}
declare class JSONFile {
    constructor(path: string);
    json(): Promise<KDFFileData>;
    /** Server runtime only, requires kdf.files.write. */
    save(value: KDFFileData): Promise<void>;
}
