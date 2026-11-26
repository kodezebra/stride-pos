import type { Catalog, NewSale, Sale, Status } from './types';

declare global {
  interface Window {
    __TAURI_INTERNALS__?: unknown;
  }
}

/** True when running inside Tauri; false in a plain browser (Vite dev). */
export function isTauri(): boolean {
  return typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;
}

const demoCatalog: Catalog = {
  version: 23,
  currency: 'UGX',
  cash_step: 100,
  tax_bp: 0,
  caps: {
    max_sale_minor: 200_000,
    max_mobile_money_minor: 50_000,
    total_mobile_money_minor: 200_000
  },
  products: [
    { id: 'p01', name: 'Sugar 2kg', price_minor: 4_500, barcode: '9876543210001', sku: 'SUG-2K', category: 'Food' },
    { id: 'p02', name: 'Bread', price_minor: 4_500, barcode: '9876543210002', sku: 'BRD-1', category: 'Food' },
    { id: 'p03', name: 'Cooking oil 1L', price_minor: 12_000, barcode: '9876543210003', sku: 'OIL-1L', category: 'Food' },
    { id: 'p04', name: 'Rice 5kg', price_minor: 18_500, barcode: '9876543210004', sku: 'RIC-5K', category: 'Food' },
    { id: 'p05', name: 'Tea 100g', price_minor: 2_000, barcode: '9876543210005', sku: 'TEA-100', category: 'Drinks' },
    { id: 'p06', name: 'Salt 500g', price_minor: 1_500, barcode: '9876543210006', sku: 'SAL-500', category: 'Food' },
    { id: 'p07', name: 'Soda 500ml', price_minor: 2_000, barcode: '9876543210007', sku: 'SOD-500', category: 'Drinks' },
    { id: 'p08', name: 'Milk 1L', price_minor: 3_500, barcode: '9876543210008', sku: 'MLK-1L', category: 'Drinks' },
    { id: 'p09', name: 'Maize flour 2kg', price_minor: 6_800, barcode: '9876543210009', sku: 'MZE-2K', category: 'Food' },
    { id: 'p10', name: 'Soap', price_minor: 3_000, barcode: '9876543210010', sku: 'SOAP-1', category: 'Household' }
  ],
  customers: [
    { id: 'c01', name: 'Amina', discount_bp: 500, credit_limit_minor: 120_000 },
    { id: 'c02', name: 'Moses', discount_bp: 0, credit_limit_minor: 80_000 },
    { id: 'c03', name: 'Grace', discount_bp: 300, credit_limit_minor: 50_000 }
  ],
  staff: [
    { id: 's01', name: 'Amina' },
    { id: 's02', name: 'Peter' }
  ]
};

let demoStatus: Status = {
  paired: true,
  offline: true,
  catalog_version: demoCatalog.version,
  last_sync: new Date(Date.now() - 2 * 60 * 1000).toISOString(),
  pending: 0,
  battery: 86
};

let demoSales: Sale[] = [];

function refreshPending(): void {
  demoStatus.pending = demoSales.filter((sale) => sale.status === 'pending').length;
}

async function mock<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  switch (command) {
    case 'get_status':
      return structuredClone(demoStatus) as T;
    case 'get_catalog':
      return structuredClone(demoCatalog) as T;
    case 'get_sales':
      return structuredClone(demoSales) as T;
    case 'record_sale': {
      const input = args?.sale as NewSale | undefined;
      if (!input) throw new Error('record_sale: missing sale');
      const unverified = input.tender === 'mtn_momo' || input.tender === 'airtel_money';
      const sale: Sale = {
        uid: crypto.randomUUID(),
        occurred_at: new Date().toISOString(),
        total_minor: input.total_minor,
        tender: input.tender,
        reference: input.reference,
        customer_id: input.customer_id,
        cashier: input.cashier,
        status: 'pending',
        unverified,
        lines: input.lines
      };
      demoSales = [sale, ...demoSales];
      refreshPending();
      return structuredClone(sale) as T;
    }
    case 'sync_now': {
      const accepted = demoSales.filter((sale) => sale.status === 'pending').length;
      demoSales = demoSales.map((sale) =>
        sale.status === 'pending' ? { ...sale, status: 'synced' as const } : sale
      );
      demoStatus = {
        ...demoStatus,
        last_sync: new Date().toISOString(),
        pending: 0
      };
      return { accepted, rejected: 0 } as T;
    }
    default:
      throw new Error(`Unknown command: ${command}`);
  }
}

async function call<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  if (isTauri()) {
    const { invoke } = await import('@tauri-apps/api/core');
    return invoke<T>(command, args);
  }
  return mock<T>(command, args);
}

export const getStatus = () => call<Status>('get_status');
export const getCatalog = () => call<Catalog>('get_catalog');
export const getSales = () => call<Sale[]>('get_sales');
export const recordSale = (sale: NewSale) => call<Sale>('record_sale', { sale });
export const syncNow = () => call<{ accepted: number; rejected: number }>('sync_now');
