import java.io.Closeable;
import java.lang.ref.Cleaner;

public class Ref<T> implements Closeable {
	private final static Cleaner cleaner = Cleaner.create();
	private RefCell<T> val;
	private final Cleaner.Cleanable clean;
	
	Ref(RefCell<T> val) {
	this.clean = cleaner.register(this, ()->{
			val.unborrow();
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
