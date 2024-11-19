import java.io.Closeable;
import java.lang.ref.Cleaner;

public class RefCell<T> implements Closeable {
	private final static Cleaner cleaner = Cleaner.create();
	// Positive = shared refs
	// -1 = mut ref
	private int refs = 0;
	final T val;
	private final Cleaner.Cleanable clean;
	private boolean relinqueshedOwnership = false;
	
	RefCell(T val) {
		if (val instanceof Closeable) {
			Closeable v = (Closeable)val;
			this.clean = cleaner.register(this, ()->{
				try {
					v.close();
				} catch (Exception e) {}
			});
		} else {
			this.clean = null;
		}
		this.val = val;
	}
	
	public synchronized Ref<T> borrow() throws BorrowException {
		if (this.refs < 0) {
			throw new BorrowException();
		}
		this.refs++;
		return new Ref<T>(this);
	}
	
	synchronized void unborrow() {
		if (this.refs < 1) {
			throw new RuntimeException("Inconsistent ref state");
		}
		this.refs--;
		
		if (this.relinqueshedOwnership == true) {
			this.close();
		}
	}
	
	public synchronized RefMut<T> borrowMut() throws BorrowException {
		if (this.refs != 0) {
			throw new BorrowException();
		}
		this.refs = -1;
		return new RefMut<T>(this);
	}
	
	public synchronized T take() throws BorrowException {
		if (this.refs != 0) {
			throw new BorrowException();
		}
		var val = this.val;
		this.refs = -2;
		return val;
	}
	
	synchronized void unborrowMut() {
		if (this.refs != -1) {
			throw new RuntimeException("Inconsistent ref state");
		}
		this.refs = 0;
		
		if (this.relinqueshedOwnership == true) {
			this.close();
		}
	}
	
	synchronized public void close() {
		if (this.val == null) {
			return;
		}

		if (this.refs == 0) {
			this.clean.clean();
		} else {
			this.relinqueshedOwnership = true;
		}
	}
}
