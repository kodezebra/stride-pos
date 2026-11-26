import { getCatalog, getSales, getStatus, recordSale, syncNow } from '$lib/api';
import type { Catalog, NewSale, Sale, Status } from '$lib/types';

class SessionStore {
  status = $state<Status | null>(null);
  catalog = $state<Catalog | null>(null);
  sales = $state<Sale[]>([]);
  cashier = $state('');
  loaded = $state(false);

  async load(): Promise<void> {
    const [status, catalog, sales] = await Promise.all([getStatus(), getCatalog(), getSales()]);
    this.status = status;
    this.catalog = catalog;
    this.sales = sales;
    if (!this.cashier && catalog.staff.length > 0) {
      this.cashier = catalog.staff[0].name;
    }
    this.loaded = true;
  }

  async sell(input: Omit<NewSale, 'cashier'>): Promise<Sale> {
    const sale = await recordSale({ ...input, cashier: this.cashier });
    this.sales = [sale, ...this.sales];
    if (this.status) {
      this.status.pending = this.sales.filter((entry) => entry.status === 'pending').length;
    }
    return sale;
  }

  async sync(): Promise<{ accepted: number; rejected: number }> {
    const result = await syncNow();
    await this.load();
    return result;
  }
}

export const session = new SessionStore();
