import java.io.Closeable;
import java.lang.ref.Cleaner;

public class RefMut<T> implements Closeable {
	private final static Cleaner cleaner = Cleaner.create();
	private RefCell<T> val;
	private final Cleaner.Cleanable clean;
	
	RefMut(RefCell<T> val) {
		this.clean = cleaner.register(this, ()->{
			val.unborrowMut();
		});
		this.val = val;
	}
	
	T get() {
		return this.val.val;
	}
	
	public void close() {
		this.val = null;
		clean.clean();
	}
}
