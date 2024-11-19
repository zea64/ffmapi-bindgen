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
		var ret = this.val.val;
		this.close();
		return ret;
	}
	
	public void close() {
		this.val = null;
		clean.clean();
	}
}
