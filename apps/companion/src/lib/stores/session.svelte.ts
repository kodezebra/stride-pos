import { getCatalog, getPlatform, getSales, getStatus, pairTill, recordSale, syncNow } from '$lib/api';
import type { Catalog, NewSale, Sale, Status, SyncResult } from '$lib/types';

class SessionStore {
  status = $state<Status | null>(null);
  catalog = $state<Catalog | null>(null);
  sales = $state<Sale[]>([]);
  cashier = $state('');
  loaded = $state(false);
  platform = $state<'browser' | 'mobile' | 'desktop'>('browser');

  async load(): Promise<void> {
    const [status, catalog, sales, platform] = await Promise.all([
      getStatus(),
      getCatalog(),
      getSales(),
      getPlatform()
    ]);
    this.platform = platform;
    this.status = status;
    this.catalog = catalog;
    this.sales = sales;
    if (!this.cashier && catalog.staff.length > 0) {
      this.cashier = catalog.staff[0].name;
    }
    this.loaded = true;
  }

  async pair(host: string, port: number, token: string): Promise<number> {
    const version = await pairTill(host, port, token);
    await this.load();
    return version;
  }

  async sell(input: Omit<NewSale, 'cashier'>): Promise<Sale> {
    const sale = await recordSale({ ...input, cashier: this.cashier });
    this.sales = [sale, ...this.sales];
    if (this.status) {
      this.status.pending = this.sales.filter((entry) => entry.status === 'pending').length;
    }
    return sale;
  }

  async sync(): Promise<SyncResult> {
    const result = await syncNow();
    await this.load();
    return result;
  }
}

export const session = new SessionStore();
